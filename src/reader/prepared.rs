use std::mem::{size_of, size_of_val};

use super::{Group, Limits, OpenOptions, Table};
use crate::ef;
use crate::format::{MAX_SYMBOLS, MAX_SYMBOL_BYTES, RAW};
use crate::sampling::{RankSampler, SamplingMethod};
use crate::{Error, Result};

/// Options for preparing a validated reader for repeated access.
#[derive(Clone, Debug)]
pub struct PrepareOptions {
    /// Maximum bytes for rank-to-group hint entries. Defaults to 4,096.
    /// Zero disables the index. This budget excludes allocator overhead.
    pub max_hint_bytes: usize,
    /// Rank reducer used by the prepared sampler.
    pub sampling: SamplingMethod,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        Self {
            max_hint_bytes: 4_096,
            sampling: SamplingMethod::MultiplyHigh,
        }
    }
}

/// A fully validated reader borrowing the checked table's groups and payload.
/// Create it with [`Table::prepare`] and keep the checked table alive for
/// the lifetime of the prepared reader.
///
/// Share this reader across calls and give each caller its own output buffer
/// and random source. Retrieval and sampling reuse initialized output storage
/// and allocate no memory. The stored ranks are the same as in the checked
/// reader. Sampling with [`SamplingMethod::Modulo`] also reproduces the
/// checked reader's sequence given the same table and random source.
#[derive(Debug)]
pub struct PreparedTable<'a> {
    payload: &'a [u8],
    groups: &'a [Group],
    ef_blocks: &'a [ef::Block],
    state: PreparedState,
}

pub(crate) struct OwnedPrepared {
    bytes: Vec<u8>,
    payload_start: usize,
    groups: Vec<Group>,
    ef_blocks: Vec<ef::Block>,
    state: PreparedState,
}

struct PreparedView<'a> {
    payload: &'a [u8],
    groups: &'a [Group],
    ef_blocks: &'a [ef::Block],
    state: &'a PreparedState,
}

#[derive(Debug)]
struct PreparedState {
    dictionary: Option<Box<DenseDictionary>>,
    hints: RankHints,
    sampler: RankSampler,
    output_len: usize,
}

#[derive(Debug)]
struct DenseDictionary {
    bytes: [[u8; MAX_SYMBOL_BYTES]; MAX_SYMBOLS],
    lengths: [u8; MAX_SYMBOLS],
}

#[derive(Debug, Default)]
struct RankHints {
    groups: Box<[u32]>,
    shift: u32,
}

impl RankHints {
    fn build(groups: &[Group], rows: u64, budget: usize) -> Result<Self> {
        let slots = (budget / size_of::<u32>()) as u64;
        if groups.len() < 2 || slots < 2 {
            return Ok(Self::default());
        }

        // Choose the smallest power-of-two bucket width with at most `slots`
        // entries. At least two groups implies at least two rows.
        let last_rank = rows - 1;
        let shift = u64::BITS - (last_rank / slots).leading_zeros();
        let entries = usize::try_from((last_rank >> shift) + 1)
            .map_err(|_| Error::LimitExceeded("prepared hint count"))?;
        let mut hints = Vec::new();
        hints
            .try_reserve_exact(entries)
            .map_err(|_| Error::LimitExceeded("prepared hint allocation"))?;

        let mut group = 0;
        for bucket in 0..entries {
            let first_rank = (bucket as u64) << shift;
            while groups[group].end_row <= first_rank {
                group += 1;
            }
            // The on-disk group count is a u32.
            hints.push(group as u32);
        }

        Ok(Self {
            groups: hints.into_boxed_slice(),
            shift,
        })
    }
}

impl Table<'_> {
    /// Verify CRC-32, decode every record, check decoded metadata and exact
    /// uniqueness, and prepare this table for repeated access.
    ///
    /// The returned reader borrows this table and uses the same immutable
    /// payload and group metadata. Validation temporarily retains decoded
    /// values for the uniqueness check. The prepared reader retains only
    /// the dense dictionary and bounded hint index as new heap allocations.
    /// Use [`PreparedTable::additional_heap_bytes`] to inspect their size.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Table::validate_all`] with uniqueness enabled.
    /// Returns [`Error::LimitExceeded`] if the required output length or
    /// hint allocation exceeds the target's limits.
    pub fn prepare(&self, options: &PrepareOptions) -> Result<PreparedTable<'_>> {
        Ok(PreparedTable {
            payload: self.payload,
            groups: &self.groups,
            ef_blocks: &self.ef_blocks,
            state: self.prepare_state(options)?,
        })
    }

    fn prepare_state(&self, options: &PrepareOptions) -> Result<PreparedState> {
        self.validate_all(true)?;

        let slack = if self.symbols.is_empty() {
            0
        } else {
            MAX_SYMBOL_BYTES - 1
        };
        let output_len = self
            .max_value_bytes()
            .checked_add(slack)
            .filter(|&length| length <= isize::MAX as usize)
            .ok_or(Error::LimitExceeded("prepared output length"))?;
        let hints = RankHints::build(&self.groups, self.len(), options.max_hint_bytes)?;
        let dictionary = (!self.symbols.is_empty()).then(|| {
            let mut dense = Box::new(DenseDictionary {
                bytes: [[0; MAX_SYMBOL_BYTES]; MAX_SYMBOLS],
                lengths: [0; MAX_SYMBOLS],
            });
            for (code, symbol) in self.symbols.iter().enumerate() {
                dense.bytes[code] = symbol.bytes;
                dense.lengths[code] = symbol.len as u8;
            }
            dense
        });

        Ok(PreparedState {
            dictionary,
            hints,
            sampler: RankSampler::new(self.len(), options.sampling),
            output_len,
        })
    }
}

impl PreparedTable<'_> {
    /// Number of stored values.
    pub fn len(&self) -> u64 {
        self.state.sampler.bound()
    }

    /// Whether this table contains zero values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Minimum initialized output length for retrieval or sampling.
    /// The returned value uses only the logical prefix of the buffer.
    pub fn output_buffer_len(&self) -> usize {
        self.state.output_len
    }

    /// Allocate and initialize storage for repeated retrieval or sampling.
    ///
    /// An allocation failure returns [`Error::LimitExceeded`].
    pub fn new_output_buffer(&self) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        initialize_output(&mut output, self.state.output_len)?;
        Ok(output)
    }

    /// Retained heap bytes for group hint entries.
    pub fn hint_bytes(&self) -> usize {
        size_of_val(self.state.hints.groups.as_ref())
    }

    /// Retained heap bytes added by preparation, including the dense dictionary
    /// and group hints. This excludes allocator overhead, the borrowed table,
    /// temporary validation allocations, and caller-owned output storage.
    pub fn additional_heap_bytes(&self) -> usize {
        let dictionary_bytes = self.state.dictionary.as_deref().map_or(0, size_of_val);
        dictionary_bytes + self.hint_bytes()
    }

    /// Decode a rank into initialized output storage without allocating.
    ///
    /// The output slice must be at least [`Self::output_buffer_len`] bytes long.
    /// The result borrows the logical UTF-8 prefix. The rest of the slice may
    /// be overwritten. Invalid ranks and short slices return typed errors.
    #[inline]
    pub fn get_into<'b>(&self, rank: u64, output: &'b mut [u8]) -> Result<&'b str> {
        if rank >= self.len() {
            return Err(Error::OutOfBounds {
                index: rank,
                len: self.len(),
            });
        }
        self.state.check_output(output.len())?;
        self.view().decode(rank, output)
    }

    /// Sample uniformly with replacement using independent uniform `u64` draws.
    ///
    /// Uses the same output contract as [`Self::get_into`]. An empty table or
    /// short output slice returns an error without drawing from the source.
    #[inline]
    pub fn sample_into<'b, F: FnMut() -> u64>(
        &self,
        next_u64: &mut F,
        output: &'b mut [u8],
    ) -> Result<&'b str> {
        self.state.check_output(output.len())?;
        let rank = self.state.sampler.sample(next_u64)?;
        self.view().decode(rank, output)
    }

    #[inline]
    fn view(&self) -> PreparedView<'_> {
        PreparedView {
            payload: self.payload,
            groups: self.groups,
            ef_blocks: self.ef_blocks,
            state: &self.state,
        }
    }
}

fn initialize_output(output: &mut Vec<u8>, len: usize) -> Result<()> {
    if output.len() < len {
        output
            .try_reserve_exact(len - output.len())
            .map_err(|_| Error::LimitExceeded("prepared output allocation"))?;
        output.resize(len, 0);
    }
    Ok(())
}

impl OwnedPrepared {
    pub(crate) fn new(bytes: Vec<u8>, limits: &Limits) -> Result<Self> {
        let table = Table::open_with_options(
            &bytes,
            &OpenOptions {
                verify_checksum: false,
                limits: limits.clone(),
            },
        )?;
        let state = table.prepare_state(&PrepareOptions {
            sampling: SamplingMethod::Modulo,
            ..PrepareOptions::default()
        })?;
        let payload_start = bytes.len() - table.payload.len();
        Ok(Self {
            groups: table.groups,
            ef_blocks: table.ef_blocks,
            bytes,
            payload_start,
            state,
        })
    }

    pub(crate) fn len(&self) -> u64 {
        self.state.sampler.bound()
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[inline]
    pub(crate) fn sample_into<'a>(
        &self,
        next_u64: &mut impl FnMut() -> u64,
        output: &'a mut Vec<u8>,
    ) -> Result<&'a str> {
        initialize_output(output, self.state.output_len)?;
        let rank = self.state.sampler.sample(next_u64)?;
        PreparedView {
            payload: &self.bytes[self.payload_start..],
            groups: &self.groups,
            ef_blocks: &self.ef_blocks,
            state: &self.state,
        }
        .decode(rank, output)
    }
}

impl PreparedState {
    #[inline]
    fn check_output(&self, provided: usize) -> Result<()> {
        if provided < self.output_len {
            return Err(Error::OutputTooSmall {
                required: self.output_len,
                provided,
            });
        }
        Ok(())
    }
}

impl PreparedView<'_> {
    #[inline]
    fn group(&self, rank: u64) -> &Group {
        let groups = self.groups;
        let hints = &self.state.hints;
        if hints.groups.is_empty() {
            return &groups[groups.partition_point(|group| group.end_row <= rank)];
        }

        let bucket = (rank >> hints.shift) as usize;
        let first = hints.groups[bucket] as usize;
        if rank < groups[first].end_row {
            return &groups[first];
        }
        // The next bucket's first group bounds this search, even when a
        // bucket overlaps many short groups.
        let end = hints
            .groups
            .get(bucket + 1)
            .map_or(groups.len(), |&group| group as usize + 1);
        let start = first + 1;
        &groups[start + groups[start..end].partition_point(|group| group.end_row <= rank)]
    }

    #[inline]
    fn decode<'b>(&self, rank: u64, output: &'b mut [u8]) -> Result<&'b str> {
        let group = self.group(rank);
        if matches!(group.codec, ef::MIXED | ef::POWER_OF_TWO) {
            let decoded = &mut output[..group.width];
            self.ef_blocks[group.offset].decode(self.payload, rank - group.first_row, decoded)?;
            return std::str::from_utf8(decoded).map_err(|_| Error::InvalidUtf8);
        }
        // Validated coverage bounds this offset within the payload. The only
        // zero-width group contains a single row, with a local rank of zero.
        let start = group.offset + (rank - group.first_row) as usize * group.width;
        let encoded = &self.payload[start..start + group.width];
        let written = if group.codec == RAW {
            output[..encoded.len()].copy_from_slice(encoded);
            encoded.len()
        } else {
            let dictionary = self
                .state
                .dictionary
                .as_ref()
                .expect("validated phrase group");
            let mut position = 0;
            for &code in encoded {
                let index = usize::from(code);
                // Every phrase adds at least one logical byte. The extra
                // output slack covers a full copy at the last starting byte.
                output[position..position + MAX_SYMBOL_BYTES]
                    .copy_from_slice(&dictionary.bytes[index]);
                position += usize::from(dictionary.lengths[index]);
            }
            position
        };
        std::str::from_utf8(&output[..written]).map_err(|_| Error::InvalidUtf8)
    }
}
