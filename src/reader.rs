use crate::ef;
use crate::format::{self, DIR_ENTRY_LEN, HEADER_LEN, MAX_SYMBOLS, MAX_SYMBOL_BYTES};
use crate::{Error, Result};
use std::collections::BTreeSet;

mod prepared;

pub(crate) use prepared::OwnedPrepared;
pub use prepared::{PrepareOptions, PreparedTable};

/// Explicit resource limits for data supplied by a caller.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Maximum size of the supplied file buffer.
    pub max_file_bytes: usize,
    /// Maximum accepted row count.
    pub max_rows: u64,
    /// Maximum decoded size of one value. Zero allows empty values and tables.
    pub max_value_bytes: usize,
    /// Maximum number of length groups.
    pub max_groups: usize,
    /// Maximum sum of decoded UTF-8 bytes reported by the file.
    pub max_decoded_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            // Accommodate the default builder's 256 MiB of values plus metadata.
            max_file_bytes: 257 * 1024 * 1024,
            max_rows: 1_000_000,
            max_value_bytes: 4096,
            max_groups: 65_536,
            max_decoded_bytes: 256 * 1024 * 1024,
        }
    }
}

/// File-opening behavior. Structural and selected-record checks always apply.
#[derive(Clone, Debug)]
pub struct OpenOptions {
    /// Check the full-file CRC-32 while opening. This reads every file byte.
    pub verify_checksum: bool,
    /// Limits independent of the untrusted file's own metadata.
    pub limits: Limits,
}
impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            verify_checksum: true,
            limits: Limits::default(),
        }
    }
}

#[derive(Clone, Debug)]
struct Symbol {
    bytes: [u8; MAX_SYMBOL_BYTES],
    len: usize,
}

#[derive(Clone, Debug)]
struct Group {
    codec: u8,
    width: usize,
    first_row: u64,
    end_row: u64,
    // Payload offset for fixed records, block index for Elias–Fano.
    offset: usize,
}

/// Storage metadata derived from a structurally valid table.
#[derive(Clone, Debug)]
pub struct TableStats {
    /// Total stored rows; distinctness is a builder or full-validation contract.
    pub row_count: u64,
    /// Complete serialized file size.
    pub file_bytes: usize,
    /// Reported total of decoded value bytes.
    pub raw_value_bytes: u64,
    /// Maximum decoded value size allowed by this file.
    pub max_value_bytes: usize,
    /// Number of flat dictionary symbols.
    pub dictionary_symbols: usize,
    /// Serialized dictionary size.
    pub dictionary_bytes: usize,
    /// Number of length groups.
    pub groups: usize,
    /// Raw UTF-8 records.
    pub raw_rows: u64,
    /// Dictionary-coded records.
    pub compressed_rows: u64,
    /// Mixed-radix Elias–Fano records.
    pub ef_mixed_rows: u64,
    /// Power-of-two radix Elias–Fano records.
    pub ef_power_of_two_rows: u64,
    /// Total serialized Elias–Fano block bytes, including indexes.
    pub ef_bytes: usize,
    /// Serialized payload bytes.
    pub payload_bytes: usize,
}

/// Results from exhaustive semantic validation.
#[derive(Clone, Debug)]
pub struct ValidationReport {
    /// Rows successfully reconstructed.
    pub row_count: u64,
    /// Sum of the reconstructed UTF-8 lengths.
    pub decoded_bytes: u64,
    /// Whether exact distinctness was also checked.
    pub uniqueness_checked: bool,
}

/// A checked reader borrowing the supplied immutable file bytes.
///
/// Opening allocates parsed metadata and retains the original table bytes by
/// reference. [`Table::open`] verifies the structure and CRC-32. Retrieval
/// checks the requested record and its UTF-8. Use [`Table::validate_all`] with
/// uniqueness enabled to establish set semantics across all rows, or
/// [`Table::prepare`] for repeated access after full validation. Ranks are local
/// to the compiled table. The caller keeps the backing bytes alive and immutable.
#[derive(Debug)]
pub struct Table<'a> {
    file: &'a [u8],
    payload: &'a [u8],
    version: u16,
    symbols: Vec<Symbol>,
    groups: Vec<Group>,
    ef_blocks: Vec<ef::Block>,
    stats: TableStats,
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    let b = bytes
        .get(at..at + 2)
        .ok_or(Error::InvalidFormat("truncated integer"))?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    let b = bytes
        .get(at..at + 4)
        .ok_or(Error::InvalidFormat("truncated integer"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn u64_at(bytes: &[u8], at: usize) -> Result<u64> {
    let b = bytes
        .get(at..at + 8)
        .ok_or(Error::InvalidFormat("truncated integer"))?;
    Ok(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}
fn as_usize(value: u64) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::LimitExceeded("integer does not fit this target"))
}
fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b)
        .ok_or(Error::InvalidFormat("section arithmetic overflow"))
}

impl<'a> Table<'a> {
    /// Open with structural validation, default resource limits and full CRC-32.
    pub fn open(bytes: &'a [u8]) -> Result<Self> {
        Self::open_with_options(bytes, &OpenOptions::default())
    }

    /// Open without the full-file checksum pass.
    ///
    /// This is memory-safe and validates structure. A damaged record may still
    /// fail when requested; corruption is never silently skipped or resampled.
    pub fn open_structural(bytes: &'a [u8]) -> Result<Self> {
        Self::open_with_options(
            bytes,
            &OpenOptions {
                verify_checksum: false,
                ..OpenOptions::default()
            },
        )
    }

    /// Open using caller-specified resource and checksum settings.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidMagic`] or [`Error::UnsupportedVersion`] for
    /// unrecognized signatures or versions, [`Error::InvalidFormat`] for
    /// structural errors, and [`Error::LimitExceeded`] for limit violations.
    /// With checksum verification enabled, a differing CRC yields
    /// [`Error::ChecksumMismatch`].
    pub fn open_with_options(bytes: &'a [u8], options: &OpenOptions) -> Result<Self> {
        let limits = &options.limits;
        if bytes.len() > limits.max_file_bytes {
            return Err(Error::LimitExceeded("file size"));
        }
        if bytes.len() < HEADER_LEN {
            return Err(Error::InvalidFormat("truncated header"));
        }
        if bytes.get(..8) != Some(format::MAGIC.as_slice()) {
            return Err(Error::InvalidMagic);
        }
        let version = u16_at(bytes, 8)?;
        let allows_empty_value = match version {
            format::VERSION_V1 => false,
            format::VERSION_V2 | format::VERSION_V3 => true,
            _ => return Err(Error::UnsupportedVersion(version)),
        };
        if u16_at(bytes, 10)? != 0 || u16_at(bytes, 38)? != 0 || u32_at(bytes, 60)? != 0 {
            return Err(Error::InvalidFormat(
                "unknown flags or nonzero reserved header bytes",
            ));
        }
        if u32_at(bytes, 12)? != HEADER_LEN as u32 {
            return Err(Error::InvalidFormat("incorrect header length"));
        }
        let row_count = u64_at(bytes, 16)?;
        let raw_value_bytes = u64_at(bytes, 24)?;
        let max_value_bytes = as_usize(u64::from(u32_at(bytes, 32)?))?;
        let symbol_count = usize::from(u16_at(bytes, 36)?);
        let group_count = as_usize(u64::from(u32_at(bytes, 40)?))?;
        let dict_bytes = as_usize(u64::from(u32_at(bytes, 44)?))?;
        let payload_bytes = as_usize(u64_at(bytes, 48)?)?;
        if row_count > limits.max_rows {
            return Err(Error::LimitExceeded("row count"));
        }
        if raw_value_bytes > limits.max_decoded_bytes {
            return Err(Error::LimitExceeded("decoded byte total"));
        }
        if max_value_bytes > limits.max_value_bytes {
            return Err(Error::LimitExceeded("decoded value size"));
        }
        if group_count > limits.max_groups {
            return Err(Error::LimitExceeded("group count"));
        }
        if symbol_count > MAX_SYMBOLS {
            return Err(Error::InvalidFormat("too many dictionary symbols"));
        }
        if group_count as u64 > row_count {
            return Err(Error::InvalidFormat("more groups than rows"));
        }
        if row_count == 0
            && (raw_value_bytes != 0
                || max_value_bytes != 0
                || symbol_count != 0
                || group_count != 0
                || dict_bytes != 0
                || payload_bytes != 0)
        {
            return Err(Error::InvalidFormat("noncanonical empty table"));
        }
        let minimum_nonempty_rows = row_count.saturating_sub(u64::from(allows_empty_value));
        if raw_value_bytes < minimum_nonempty_rows
            || raw_value_bytes < max_value_bytes as u64
            || u128::from(raw_value_bytes) > u128::from(row_count) * max_value_bytes as u128
        {
            return Err(Error::InvalidFormat("inconsistent decoded length metadata"));
        }
        let dict_end = add(HEADER_LEN, dict_bytes)?;
        let dir_bytes = group_count
            .checked_mul(DIR_ENTRY_LEN)
            .ok_or(Error::InvalidFormat("directory size overflow"))?;
        let payload_start = add(dict_end, dir_bytes)?;
        if add(payload_start, payload_bytes)? != bytes.len() {
            return Err(Error::InvalidFormat(
                "section lengths do not cover file exactly",
            ));
        }
        let payload = &bytes[payload_start..];

        let dictionary = bytes
            .get(HEADER_LEN..dict_end)
            .ok_or(Error::InvalidFormat("truncated dictionary"))?;
        let mut symbols: Vec<Symbol> = Vec::with_capacity(symbol_count);
        let mut pos = 0usize;
        for _ in 0..symbol_count {
            let len = usize::from(
                *dictionary
                    .get(pos)
                    .ok_or(Error::InvalidFormat("missing symbol length"))?,
            );
            pos = add(pos, 1)?;
            if !(1..=MAX_SYMBOL_BYTES).contains(&len) {
                return Err(Error::InvalidFormat("invalid symbol expansion length"));
            }
            let end = add(pos, len)?;
            let value = dictionary
                .get(pos..end)
                .ok_or(Error::InvalidFormat("truncated symbol"))?;
            if symbols.iter().any(|s| &s.bytes[..s.len] == value) {
                return Err(Error::InvalidFormat("duplicate dictionary expansion"));
            }
            let mut storage = [0; MAX_SYMBOL_BYTES];
            storage[..len].copy_from_slice(value);
            symbols.push(Symbol {
                bytes: storage,
                len,
            });
            pos = end;
        }
        if pos != dictionary.len() {
            return Err(Error::InvalidFormat("unused dictionary bytes"));
        }

        let mut groups = Vec::new();
        groups
            .try_reserve_exact(group_count)
            .map_err(|_| Error::LimitExceeded("group directory allocation"))?;
        let mut next_row = 0u64;
        let mut next_byte = 0usize;
        let mut previous_key: Option<(u8, usize)> = None;
        let mut raw_rows = 0u64;
        let mut compressed_rows = 0u64;
        let mut ef_mixed_rows = 0u64;
        let mut ef_power_of_two_rows = 0u64;
        let mut ef_bytes = 0usize;
        let mut ef_blocks = Vec::new();
        let mut decoded_floor = 0u64;
        for i in 0..group_count {
            let entry_start = dict_end + i * DIR_ENTRY_LEN;
            let entry = &bytes[entry_start..entry_start + DIR_ENTRY_LEN];
            let codec = entry[0];
            let is_ef = matches!(codec, ef::MIXED | ef::POWER_OF_TWO);
            if codec != format::RAW
                && codec != format::PHRASES
                && !(version == format::VERSION_V3 && is_ef)
            {
                return Err(Error::InvalidFormat("unknown record codec"));
            }
            if entry[1..4] != [0, 0, 0] {
                return Err(Error::InvalidFormat("nonzero reserved group bytes"));
            }
            let width = as_usize(u64::from(u32_at(entry, 4)?))?;
            let count = u64_at(entry, 8)?;
            if count == 0 {
                return Err(Error::InvalidFormat("empty group"));
            }
            if width == 0 && (!allows_empty_value || codec != format::RAW || count != 1) {
                return Err(Error::InvalidFormat("invalid zero-width record group"));
            }
            if width > max_value_bytes {
                return Err(Error::InvalidFormat(
                    "encoded width exceeds decoded maximum",
                ));
            }
            let key = (codec, width);
            if previous_key.is_some_and(|old| old >= key) {
                return Err(Error::InvalidFormat("group keys are not strictly ordered"));
            }
            previous_key = Some(key);
            if codec == format::PHRASES && symbols.is_empty() {
                return Err(Error::InvalidFormat(
                    "phrase-coded records have no dictionary",
                ));
            }
            let end_row = next_row
                .checked_add(count)
                .ok_or(Error::InvalidFormat("row count overflow"))?;
            if end_row > row_count {
                return Err(Error::InvalidFormat("group exceeds declared rows"));
            }
            let width_times_count = (width as u64)
                .checked_mul(count)
                .ok_or(Error::InvalidFormat("group payload size overflow"))?;
            decoded_floor = decoded_floor
                .checked_add(width_times_count)
                .ok_or(Error::InvalidFormat("group decoded byte total overflow"))?;
            if version == format::VERSION_V3 && decoded_floor > raw_value_bytes {
                return Err(Error::InvalidFormat(
                    "group decoded bytes exceed declared total",
                ));
            }
            let (offset, group_bytes) = if is_ef {
                let (block, bytes) = ef::Block::open(payload, next_byte, width, count, codec)?;
                let index = ef_blocks.len();
                ef_blocks
                    .try_reserve(1)
                    .map_err(|_| Error::LimitExceeded("Elias–Fano metadata allocation"))?;
                ef_blocks.push(block);
                ef_bytes += bytes;
                if codec == ef::MIXED {
                    ef_mixed_rows += count;
                } else {
                    ef_power_of_two_rows += count;
                }
                (index, bytes)
            } else {
                if codec == format::RAW {
                    raw_rows += count;
                } else {
                    compressed_rows += count;
                }
                (next_byte, as_usize(width_times_count)?)
            };
            let end_byte = add(next_byte, group_bytes)?;
            if end_byte > payload_bytes {
                return Err(Error::InvalidFormat("group exceeds declared payload"));
            }
            groups.push(Group {
                codec,
                width,
                first_row: next_row,
                end_row,
                offset,
            });
            next_row = end_row;
            next_byte = end_byte;
        }
        if next_row != row_count || next_byte != payload_bytes {
            return Err(Error::InvalidFormat("group coverage does not match header"));
        }
        // The sorted directory places the optional singleton empty value first.
        let nonempty_rows =
            row_count - u64::from(groups.first().is_some_and(|group| group.width == 0));
        if allows_empty_value
            && (raw_value_bytes < nonempty_rows
                || u128::from(raw_value_bytes)
                    > u128::from(nonempty_rows) * max_value_bytes as u128)
        {
            return Err(Error::InvalidFormat("inconsistent decoded length metadata"));
        }
        if compressed_rows == 0 && !symbols.is_empty() {
            return Err(Error::InvalidFormat("dictionary without phrase groups"));
        }
        if ef_blocks.is_empty() && raw_value_bytes < payload_bytes as u64 {
            return Err(Error::InvalidFormat(
                "decoded byte total is smaller than encoded payload",
            ));
        }
        if compressed_rows == 0 {
            let actual_max = groups.iter().map(|group| group.width).max().unwrap_or(0);
            if raw_value_bytes != decoded_floor || max_value_bytes != actual_max {
                return Err(Error::InvalidFormat(
                    "decoded length metadata does not match group widths",
                ));
            }
        }
        let table = Self {
            file: bytes,
            payload,
            version,
            symbols,
            groups,
            ef_blocks,
            stats: TableStats {
                row_count,
                file_bytes: bytes.len(),
                raw_value_bytes,
                max_value_bytes,
                dictionary_symbols: symbol_count,
                dictionary_bytes: dict_bytes,
                groups: group_count,
                raw_rows,
                compressed_rows,
                ef_mixed_rows,
                ef_power_of_two_rows,
                ef_bytes,
                payload_bytes,
            },
        };
        if options.verify_checksum {
            table.verify_checksum()?;
        }
        Ok(table)
    }

    /// On-disk format version of this table.
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Number of stored values.
    pub fn len(&self) -> u64 {
        self.stats.row_count
    }
    /// Whether the table has no records.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Maximum original value length declared by the validated header.
    /// Use [`Table::is_empty`] to distinguish an empty table from a table whose
    /// sole value is the empty string.
    pub fn max_value_bytes(&self) -> usize {
        self.stats.max_value_bytes
    }
    /// Structural storage statistics; decoded totals are verified by `validate_all`.
    pub fn stats(&self) -> TableStats {
        self.stats.clone()
    }

    /// Bytes retained in parsed group, dictionary, and codec metadata allocations.
    ///
    /// Includes spare allocation capacity. Excludes allocator overhead,
    /// the backing bytes, and output scratch space.
    pub fn metadata_heap_bytes(&self) -> usize {
        use std::mem::size_of;
        self.groups.capacity() * size_of::<Group>()
            + self.symbols.capacity() * size_of::<Symbol>()
            + self.ef_blocks.capacity() * size_of::<ef::Block>()
    }

    fn group_for_rank(&self, rank: u64) -> Result<&Group> {
        if rank >= self.len() {
            return Err(Error::OutOfBounds {
                index: rank,
                len: self.len(),
            });
        }
        let group_id = self.groups.partition_point(|g| g.end_row <= rank);
        self.groups
            .get(group_id)
            .ok_or(Error::InvalidFormat("rank has no group"))
    }

    fn fixed_record(&self, group: &Group, rank: u64) -> Result<&'a [u8]> {
        let local = as_usize(rank - group.first_row)?;
        let displacement = local
            .checked_mul(group.width)
            .ok_or(Error::InvalidFormat("record offset overflow"))?;
        let start = add(group.offset, displacement)?;
        let end = add(start, group.width)?;
        self.payload
            .get(start..end)
            .ok_or(Error::InvalidFormat("record outside payload"))
    }

    /// Reconstruct one stored rank into reusable storage and borrow the result.
    ///
    /// The buffer does not grow when its capacity is at least
    /// [`Table::max_value_bytes`].
    ///
    /// The output is cleared before decoding. Its contents are unspecified if
    /// decoding returns an error. A successful result is always complete UTF-8.
    ///
    /// # Errors
    ///
    /// Returns [`Error::OutOfBounds`] for a missing rank and propagates format,
    /// resource, and UTF-8 errors from decoding the selected record.
    pub fn get_into<'b>(&self, rank: u64, output: &'b mut Vec<u8>) -> Result<&'b str> {
        output.clear();
        let group = self.group_for_rank(rank)?;
        if matches!(group.codec, ef::MIXED | ef::POWER_OF_TWO) {
            output
                .try_reserve_exact(group.width)
                .map_err(|_| Error::LimitExceeded("output allocation"))?;
            output.resize(group.width, 0);
            self.ef_blocks[group.offset].decode(self.payload, rank - group.first_row, output)?;
            return std::str::from_utf8(output).map_err(|_| Error::InvalidUtf8);
        }
        let encoded = self.fixed_record(group, rank)?;
        let reserve_bytes = if group.codec == format::RAW {
            encoded.len()
        } else {
            encoded
                .len()
                .saturating_mul(MAX_SYMBOL_BYTES)
                .min(self.max_value_bytes())
        };
        if output.capacity() < reserve_bytes {
            output
                .try_reserve_exact(reserve_bytes)
                .map_err(|_| Error::LimitExceeded("output allocation"))?;
        }
        if group.codec == format::RAW {
            output.extend_from_slice(encoded);
        } else {
            for &code in encoded {
                let symbol = self
                    .symbols
                    .get(usize::from(code))
                    .ok_or(Error::InvalidFormat("undefined dictionary symbol"))?;
                let expanded_len = output
                    .len()
                    .checked_add(symbol.len)
                    .ok_or(Error::InvalidFormat("decoded record size overflow"))?;
                if expanded_len > self.max_value_bytes() {
                    return Err(Error::LimitExceeded(
                        "decoded record exceeds maximum value size",
                    ));
                }
                output.extend_from_slice(&symbol.bytes[..symbol.len]);
            }
        }
        std::str::from_utf8(output).map_err(|_| Error::InvalidUtf8)
    }

    /// Reconstruct one stored rank as an owned string.
    pub fn get(&self, rank: u64) -> Result<String> {
        let mut output = Vec::new();
        self.get_into(rank, &mut output)?;
        String::from_utf8(output).map_err(|_| Error::InvalidUtf8)
    }

    /// Borrow a raw record directly; returns `None` for a compressed record.
    ///
    /// The raw record is checked for complete UTF-8 before it is returned.
    /// A stored empty value returns `Some("")`.
    pub fn get_raw(&self, rank: u64) -> Result<Option<&'a str>> {
        let group = self.group_for_rank(rank)?;
        if group.codec == format::RAW {
            let encoded = self.fixed_record(group, rank)?;
            Ok(Some(
                std::str::from_utf8(encoded).map_err(|_| Error::InvalidUtf8)?,
            ))
        } else {
            Ok(None)
        }
    }

    /// Sample a stored rank uniformly with replacement using an independent
    /// uniform `u64` source.
    ///
    /// The caller chooses the RNG, including whether it is cryptographic.
    /// Each rank maps to a distinct value in a table built by this crate or
    /// verified with [`Table::validate_all`] and uniqueness enabled. A malformed
    /// selected record returns its decoding error without drawing again.
    /// An empty table returns [`Error::EmptyTable`] before requesting a draw.
    pub fn sample_into<'b, F: FnMut() -> u64>(
        &self,
        next_u64: &mut F,
        output: &'b mut Vec<u8>,
    ) -> Result<&'b str> {
        let rank = crate::uniform_below(self.len(), next_u64)?;
        self.get_into(rank, output)
    }

    /// Verify the full-file IEEE CRC-32. This scans the entire supplied buffer.
    pub fn verify_checksum(&self) -> Result<()> {
        let expected = u32_at(self.file, 56)?;
        let actual = format::checksum(self.file);
        if expected != actual {
            return Err(Error::ChecksumMismatch { expected, actual });
        }
        Ok(())
    }

    /// Verify CRC, decode all rows, and optionally verify exact distinctness.
    ///
    /// Uniqueness verification retains decoded values in a set and therefore
    /// uses memory proportional to decoded data; normal opening does not do so.
    pub fn validate_all(&self, check_unique: bool) -> Result<ValidationReport> {
        self.verify_checksum()?;
        let mut output = Vec::new();
        let mut decoded_bytes = 0u64;
        let mut actual_max = 0usize;
        let mut unique = if check_unique {
            Some(BTreeSet::new())
        } else {
            None
        };
        for rank in 0..self.len() {
            let value = self.get_into(rank, &mut output)?;
            decoded_bytes = decoded_bytes
                .checked_add(value.len() as u64)
                .ok_or(Error::InvalidFormat("decoded byte sum overflow"))?;
            if decoded_bytes > self.stats.raw_value_bytes {
                return Err(Error::InvalidFormat("decoded byte total exceeds header"));
            }
            actual_max = actual_max.max(value.len());
            if let Some(set) = &mut unique {
                if !set.insert(value.to_owned()) {
                    return Err(Error::DuplicateValue(value.to_owned()));
                }
            }
        }
        if decoded_bytes != self.stats.raw_value_bytes || actual_max != self.max_value_bytes() {
            return Err(Error::InvalidFormat("decoded lengths disagree with header"));
        }
        Ok(ValidationReport {
            row_count: self.len(),
            decoded_bytes,
            uniqueness_checked: check_unique,
        })
    }
}
