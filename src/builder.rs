//! Deterministic offline construction and bounded phrase training.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::ef;
use crate::format::{
    checksum, DIR_ENTRY_LEN, HEADER_LEN, MAGIC, MAX_SYMBOLS, MAX_SYMBOL_BYTES, PHRASES, RAW,
    VERSION_V2, VERSION_V3,
};
use crate::{Error, Result};

// Bound scratch space and work per value in the phrase encoder.
const MAX_PHRASE_VALUE_BYTES: usize = 4_096;

/// How the builder selects a representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildMode {
    /// Store UTF-8 bytes directly in groups of equal byte length.
    Raw,
    /// Compare raw storage with a bounded set of phrase dictionaries.
    ///
    /// Values longer than 4,096 bytes use raw storage. The complete raw layout
    /// is always a candidate and wins ties against a compressed representation.
    Phrase,
    /// Try mixed-radix Elias–Fano by original value length with raw fallback.
    EliasFanoMixed,
    /// Try power-of-two radix Elias–Fano by original value length with raw fallback.
    EliasFanoPowerOfTwo,
    /// Compare complete raw, phrase, and both Elias–Fano representations.
    Compact,
}

/// Treatment of values that are equal byte for byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DuplicatePolicy {
    /// Return [`Error::DuplicateValue`] when an input value is repeated.
    Reject,
    /// Keep one instance of each distinct value.
    Deduplicate,
}

/// Resource limits and representation choices for [`build`].
#[derive(Clone, Debug)]
pub struct BuildOptions {
    /// Raw storage or measured selection among raw and compressed candidates.
    pub mode: BuildMode,
    /// Whether duplicate input values are rejected or removed.
    pub duplicate_policy: DuplicatePolicy,
    /// Maximum UTF-8 byte length of one input value. Defaults to 4,096.
    /// Zero allows only empty values, and the limit must fit in `u32`.
    pub max_value_bytes: usize,
    /// Maximum values used for phrase training; default 4,096.
    ///
    /// Zero disables phrase training. Training uses eligible values at
    /// deterministic positions in the sorted unique input.
    pub training_values: usize,
    /// Maximum original value bytes in the training sample; default 262,144.
    ///
    /// Zero disables phrase training. Only whole, nonempty values of at most
    /// 4,096 bytes take part in training.
    pub training_bytes: usize,
    /// Maximum literal and phrase symbols combined; must be 1 through 256.
    ///
    /// If the observed literal alphabet does not fit, raw storage is used.
    pub max_symbols: usize,
    /// Maximum input row count before deduplication; default 1,000,000.
    pub max_input_rows: usize,
    /// Maximum cumulative input UTF-8 bytes, including duplicates.
    ///
    /// Defaults to 256 MiB. This limits retained source data, not total process
    /// memory: sorting, candidate payloads and training also need memory.
    pub max_input_bytes: usize,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            mode: BuildMode::Compact,
            duplicate_policy: DuplicatePolicy::Reject,
            max_value_bytes: 4_096,
            training_values: 4_096,
            training_bytes: 262_144,
            max_symbols: MAX_SYMBOLS,
            max_input_rows: 1_000_000,
            max_input_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Measurements from one completed build.
#[derive(Clone, Debug)]
pub struct BuildStats {
    /// Number of unique rows in the resulting table.
    pub row_count: u64,
    /// Number of rows consumed before deduplication.
    pub input_rows: usize,
    /// Number of repeated input rows removed by deduplication.
    pub duplicate_rows: usize,
    /// Sum of original UTF-8 value lengths after deduplication.
    pub raw_value_bytes: u64,
    /// Complete serialized file length, including metadata and dictionary.
    pub packed_bytes: usize,
    /// Number of dictionary entries retained in the selected file.
    pub dictionary_symbols: usize,
    /// Serialized dictionary section length, including entry length bytes.
    pub dictionary_bytes: usize,
    /// Number of groups in the selected file.
    pub groups: usize,
    /// Number of values stored directly as UTF-8 bytes.
    pub raw_rows: u64,
    /// Number of values stored as phrase-symbol codes.
    pub compressed_rows: u64,
    /// Number of values stored as mixed-radix Elias–Fano integers.
    pub ef_mixed_rows: u64,
    /// Number of values stored as power-of-two radix Elias–Fano integers.
    pub ef_power_of_two_rows: u64,
    /// Total bytes in Elias–Fano blocks, including block headers and indexes.
    pub ef_bytes: usize,
    /// Longest original value in bytes, or zero if there are no nonempty values.
    pub max_value_bytes: usize,
    /// Number of complete file candidates compared, including the raw baseline.
    pub candidate_models: usize,
    /// Number of whole values in the actual phrase-training sample.
    pub training_values: usize,
}

/// A complete portable table and the measurements from building it.
#[derive(Clone, Debug)]
pub struct BuildOutput {
    /// Version 3 file bytes with Elias–Fano, or version 2 otherwise, including CRC-32.
    pub bytes: Vec<u8>,
    /// Size, row and model-selection measurements.
    pub stats: BuildStats,
}

/// Build a deterministic table of exact UTF-8 values.
///
/// Input bytes are stored as supplied, including empty strings, whitespace, and
/// control characters. A fixed writer and build options produce identical bytes
/// for the same distinct values regardless of input order. When deduplicating,
/// repeated values produce the same table bytes as one copy of each value.
/// Codec selection and ranks can change when rebuilding with a different writer.
/// Compact mode compares complete file sizes and uses the raw layout when it
/// is smallest.
///
/// # Errors
///
/// Returns an error for invalid options, exceeded resource limits, or a
/// duplicate when `duplicate_policy` is [`DuplicatePolicy::Reject`].
pub fn build<I, S>(values: I, options: &BuildOptions) -> Result<BuildOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    build_records(values.into_iter().map(Ok), options, |value| {
        value.as_ref().to_owned()
    })
}

pub(crate) fn build_records<I, S>(
    values: I,
    options: &BuildOptions,
    mut own: impl FnMut(S) -> String,
) -> Result<BuildOutput>
where
    I: IntoIterator<Item = Result<S>>,
    S: AsRef<str>,
{
    validate_options(options)?;
    let mut source = Vec::<String>::new();
    let mut input_rows = 0usize;
    let mut input_bytes = 0usize;
    for item in values {
        let item = item?;
        let value = item.as_ref();
        if value.len() > options.max_value_bytes {
            return Err(Error::LimitExceeded("input value byte length"));
        }
        input_rows = input_rows
            .checked_add(1)
            .ok_or(Error::LimitExceeded("input row count"))?;
        input_bytes = input_bytes
            .checked_add(value.len())
            .ok_or(Error::LimitExceeded("cumulative input byte length"))?;
        if input_rows > options.max_input_rows {
            return Err(Error::LimitExceeded("input row count"));
        }
        if input_bytes > options.max_input_bytes {
            return Err(Error::LimitExceeded("cumulative input byte length"));
        }
        source.push(own(item));
    }
    source.sort_unstable();
    if options.duplicate_policy == DuplicatePolicy::Reject {
        if let Some(pair) = source.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(Error::DuplicateValue(pair[0].clone()));
        }
    } else {
        source.dedup();
    }
    let duplicate_rows = input_rows - source.len();
    let row_count = as_u64(source.len(), "unique row count")?;
    let mut raw_value_bytes = 0u64;
    let mut max_value_bytes = 0usize;
    for value in &source {
        raw_value_bytes = raw_value_bytes
            .checked_add(as_u64(value.len(), "value byte length")?)
            .ok_or(Error::LimitExceeded("total original value bytes"))?;
        max_value_bytes = max_value_bytes.max(value.len());
    }
    let info = SourceInfo {
        row_count,
        raw_value_bytes,
        max_value_bytes,
    };
    let mut best = serialize(&source, &[], &info)?;
    let mut candidate_models = 1usize;
    let mut training_values = 0usize;
    if matches!(options.mode, BuildMode::Compact | BuildMode::Phrase) && info.max_value_bytes != 0 {
        let training = train(&source, options);
        training_values = training.sample_values;
        for dictionary in training.models {
            let candidate = serialize(&source, &dictionary, &info)?;
            candidate_models += 1;
            if candidate.bytes.len() < best.bytes.len() {
                best = candidate;
            }
        }
    }
    let ef_codecs: &[u8] = match options.mode {
        BuildMode::Compact => &[ef::MIXED, ef::POWER_OF_TWO],
        BuildMode::EliasFanoMixed => &[ef::MIXED],
        BuildMode::EliasFanoPowerOfTwo => &[ef::POWER_OF_TWO],
        BuildMode::Raw | BuildMode::Phrase => &[],
    };
    if !ef_codecs.is_empty() && source.len() > 1 {
        let mut widths: BTreeMap<usize, Vec<&[u8]>> = BTreeMap::new();
        for value in &source {
            widths
                .entry(value.len())
                .or_default()
                .push(value.as_bytes());
        }
        for &codec in ef_codecs {
            if let Some(candidate) = serialize_ef(&widths, codec, &info)? {
                candidate_models += 1;
                if candidate.bytes.len() < best.bytes.len() {
                    best = candidate;
                }
            }
        }
    }
    let stats = BuildStats {
        row_count,
        input_rows,
        duplicate_rows,
        raw_value_bytes,
        packed_bytes: best.bytes.len(),
        dictionary_symbols: best.dictionary_symbols,
        dictionary_bytes: best.dictionary_bytes,
        groups: best.groups,
        raw_rows: best.raw_rows,
        compressed_rows: best.compressed_rows,
        ef_mixed_rows: best.ef_mixed_rows,
        ef_power_of_two_rows: best.ef_power_of_two_rows,
        ef_bytes: best.ef_bytes,
        max_value_bytes,
        candidate_models,
        training_values,
    };
    Ok(BuildOutput {
        bytes: best.bytes,
        stats,
    })
}

fn validate_options(options: &BuildOptions) -> Result<()> {
    if options.max_symbols == 0 || options.max_symbols > MAX_SYMBOLS {
        return Err(Error::InvalidOptions("max_symbols must be 1 through 256"));
    }
    if u32::try_from(options.max_value_bytes).is_err() {
        return Err(Error::InvalidOptions("max_value_bytes must fit in u32"));
    }
    Ok(())
}

struct SourceInfo {
    row_count: u64,
    raw_value_bytes: u64,
    max_value_bytes: usize,
}

struct Training {
    models: Vec<Vec<Vec<u8>>>,
    sample_values: usize,
}

fn phrase_eligible(value: &[u8]) -> bool {
    (1..=MAX_PHRASE_VALUE_BYTES).contains(&value.len())
}

fn train(source: &[String], options: &BuildOptions) -> Training {
    let mut result = Training {
        models: Vec::new(),
        sample_values: 0,
    };
    if options.training_values == 0 || options.training_bytes == 0 {
        return result;
    }
    let mut observed = [false; 256];
    for bytes in source
        .iter()
        .map(|value| value.as_bytes())
        .filter(|bytes| phrase_eligible(bytes))
    {
        for &byte in bytes {
            observed[usize::from(byte)] = true;
        }
    }
    let mut dictionary: Vec<Vec<u8>> = observed
        .iter()
        .enumerate()
        .filter(|(_, present)| **present)
        .map(|(byte, _)| vec![byte as u8])
        .collect();
    if dictionary.is_empty() || dictionary.len() >= options.max_symbols {
        return result;
    }
    let literal_count = dictionary.len();
    let mut literal_codes = [0u8; 256];
    for (code, symbol) in dictionary.iter().enumerate() {
        literal_codes[usize::from(symbol[0])] = code as u8;
    }
    let requested = source.len().min(options.training_values);
    let mut sequences: Vec<Vec<u8>> = Vec::new();
    let mut sample_bytes = 0usize;
    for sample_index in 0..requested {
        // The product is evaluated at 128 bits so usize multiplication cannot
        // overflow. The quotient is strictly below source.len().
        let index = (sample_index as u128 * source.len() as u128 / requested as u128) as usize;
        let bytes = source[index].as_bytes();
        if !phrase_eligible(bytes) || bytes.len() > options.training_bytes - sample_bytes {
            continue;
        }
        sample_bytes += bytes.len();
        sequences.push(
            bytes
                .iter()
                .map(|byte| literal_codes[usize::from(*byte)])
                .collect(),
        );
    }
    result.sample_values = sequences.len();
    if sequences.is_empty() {
        return result;
    }
    let slots = options.max_symbols - literal_count;
    let checkpoints = [1, 2, 3, 4].map(|step| (slots * step).div_ceil(4));
    let mut known: BTreeSet<Vec<u8>> = dictionary.iter().cloned().collect();
    let mut counts = vec![0usize; 65_536];
    let mut last_checkpoint = 0usize;
    while dictionary.len() < options.max_symbols {
        count_pairs(&sequences, &mut counts);
        let mut best_gain = 0usize;
        let mut best_key = 0usize;
        let mut best_symbol = [0u8; MAX_SYMBOL_BYTES];
        let mut best_length = 0usize;
        for (key, &count) in counts.iter().enumerate() {
            if count == 0 {
                continue;
            }
            let left = &dictionary[key >> 8];
            let right = &dictionary[key & 255];
            let length = left.len() + right.len();
            if length > MAX_SYMBOL_BYTES || count <= length + 1 {
                continue;
            }
            let mut symbol = [0u8; MAX_SYMBOL_BYTES];
            symbol[..left.len()].copy_from_slice(left);
            symbol[left.len()..length].copy_from_slice(right);
            if known.contains(&symbol[..length]) {
                continue;
            }
            let gain = count - (length + 1);
            if gain > best_gain
                || (gain == best_gain
                    && (symbol[..length] < best_symbol[..best_length]
                        || (symbol[..length] == best_symbol[..best_length] && key < best_key)))
            {
                best_gain = gain;
                best_key = key;
                best_symbol = symbol;
                best_length = length;
            }
        }
        if best_gain == 0 {
            break;
        }
        let new_code = dictionary.len() as u8;
        let left = (best_key >> 8) as u8;
        let right = (best_key & 255) as u8;
        let merged = merge_pair(&mut sequences, left, right, new_code);
        debug_assert_eq!(merged, counts[best_key]);
        let symbol = best_symbol[..best_length].to_vec();
        known.insert(symbol.clone());
        dictionary.push(symbol);
        let learned = dictionary.len() - literal_count;
        if checkpoints.contains(&learned) {
            result.models.push(dictionary.clone());
            last_checkpoint = learned;
        }
    }
    let learned = dictionary.len() - literal_count;
    if learned > 0 && learned != last_checkpoint {
        result.models.push(dictionary);
    }
    result
}

/// Count realizable non-overlapping uses of each pair independently.
fn count_pairs(sequences: &[Vec<u8>], counts: &mut [usize]) {
    counts.fill(0);
    for sequence in sequences {
        let mut start = 0usize;
        while start < sequence.len() {
            let code = sequence[start];
            let mut end = start + 1;
            while end < sequence.len() && sequence[end] == code {
                end += 1;
            }
            counts[(usize::from(code) << 8) | usize::from(code)] += (end - start) / 2;
            if end < sequence.len() {
                counts[(usize::from(code) << 8) | usize::from(sequence[end])] += 1;
            }
            start = end;
        }
    }
}

fn merge_pair(sequences: &mut [Vec<u8>], left: u8, right: u8, replacement: u8) -> usize {
    let mut merged = 0usize;
    for sequence in sequences {
        let mut read = 0usize;
        let mut write = 0usize;
        while read < sequence.len() {
            if read + 1 < sequence.len() && sequence[read] == left && sequence[read + 1] == right {
                sequence[write] = replacement;
                merged += 1;
                read += 2;
            } else {
                sequence[write] = sequence[read];
                read += 1;
            }
            write += 1;
        }
        sequence.truncate(write);
    }
    merged
}

struct Encoder<'a> {
    dictionary: &'a [Vec<u8>],
    starts: [Vec<u8>; 256],
    costs: Vec<usize>,
    choices: Vec<u8>,
}

impl<'a> Encoder<'a> {
    fn new(dictionary: &'a [Vec<u8>]) -> Self {
        let mut starts: [Vec<u8>; 256] = std::array::from_fn(|_| Vec::new());
        for (code, symbol) in dictionary.iter().enumerate() {
            starts[usize::from(symbol[0])].push(code as u8);
        }
        Self {
            dictionary,
            starts,
            costs: Vec::new(),
            choices: Vec::new(),
        }
    }

    fn encode_if_shorter(&mut self, input: &[u8]) -> Option<Vec<u8>> {
        self.costs.resize(input.len() + 1, 0);
        self.choices.resize(input.len(), 0);
        self.costs[input.len()] = 0;
        for position in (0..input.len()).rev() {
            let mut best = usize::MAX;
            let mut selected = 0u8;
            for &code in &self.starts[usize::from(input[position])] {
                let symbol = &self.dictionary[usize::from(code)];
                if input[position..].starts_with(symbol) {
                    let cost = self.costs[position + symbol.len()] + 1;
                    if cost < best {
                        best = cost;
                        selected = code;
                    }
                }
            }
            // Every observed literal byte has a symbol in training models.
            // Keeping this defensive fallback also prevents overflow if this
            // private helper is used with an incomplete test dictionary.
            if best == usize::MAX {
                return None;
            }
            self.costs[position] = best;
            self.choices[position] = selected;
        }
        if self.costs[0] >= input.len() {
            return None;
        }
        let mut output = Vec::with_capacity(self.costs[0]);
        let mut position = 0usize;
        while position < input.len() {
            let code = self.choices[position];
            output.push(code);
            position += self.dictionary[usize::from(code)].len();
        }
        Some(output)
    }
}

struct Serialized {
    bytes: Vec<u8>,
    dictionary_symbols: usize,
    dictionary_bytes: usize,
    groups: usize,
    raw_rows: u64,
    compressed_rows: u64,
    ef_mixed_rows: u64,
    ef_power_of_two_rows: u64,
    ef_bytes: usize,
}

enum GroupData<'a> {
    Fixed(Vec<Cow<'a, [u8]>>),
    EliasFano { bytes: Vec<u8>, rows: usize },
}

impl GroupData<'_> {
    fn rows(&self) -> usize {
        match self {
            Self::Fixed(records) => records.len(),
            Self::EliasFano { rows, .. } => *rows,
        }
    }

    fn byte_len(&self, width: usize) -> Result<usize> {
        match self {
            Self::Fixed(records) => width
                .checked_mul(records.len())
                .ok_or(Error::LimitExceeded("group payload byte length")),
            Self::EliasFano { bytes, .. } => Ok(bytes.len()),
        }
    }
}

fn serialize_ef(
    widths: &BTreeMap<usize, Vec<&[u8]>>,
    codec: u8,
    info: &SourceInfo,
) -> Result<Option<Serialized>> {
    let mut groups = BTreeMap::new();
    let mut has_ef = false;
    for (&width, values) in widths {
        let raw_len = width
            .checked_mul(values.len())
            .ok_or(Error::LimitExceeded("group payload byte length"))?;
        let block = if (1..=ef::MAX_WIDTH).contains(&width) && values.len() > 1 {
            ef::encode(values, codec).filter(|bytes| bytes.len() < raw_len)
        } else {
            None
        };
        if let Some(bytes) = block {
            has_ef = true;
            groups.insert(
                (codec, width),
                GroupData::EliasFano {
                    bytes,
                    rows: values.len(),
                },
            );
        } else {
            let records = values.iter().copied().map(Cow::Borrowed).collect();
            groups.insert((RAW, width), GroupData::Fixed(records));
        }
    }
    if !has_ef {
        return Ok(None);
    }
    write_file(groups, &[], info).map(Some)
}

fn serialize(source: &[String], dictionary: &[Vec<u8>], info: &SourceInfo) -> Result<Serialized> {
    let mut groups: BTreeMap<(u8, usize), Vec<Cow<'_, [u8]>>> = BTreeMap::new();
    let mut used = [false; MAX_SYMBOLS];
    let mut encoder = Encoder::new(dictionary);
    for value in source {
        let input = value.as_bytes();
        let encoded = if !dictionary.is_empty() && phrase_eligible(input) {
            encoder.encode_if_shorter(input)
        } else {
            None
        };
        if let Some(encoded) = encoded {
            for &code in &encoded {
                used[usize::from(code)] = true;
            }
            groups
                .entry((PHRASES, encoded.len()))
                .or_default()
                .push(Cow::Owned(encoded));
        } else {
            groups
                .entry((RAW, input.len()))
                .or_default()
                .push(Cow::Borrowed(input));
        }
    }

    // Raw rows need no dictionary. Remove every symbol unused by compressed
    // rows, preserving symbol order, then rewrite their compact byte codes.
    let mut remap = [0u8; MAX_SYMBOLS];
    let mut retained = Vec::new();
    for (code, symbol) in dictionary.iter().enumerate() {
        if used[code] {
            remap[code] = retained.len() as u8;
            retained.push(symbol);
        }
    }
    for (&(codec, _), records) in &mut groups {
        if codec == PHRASES {
            for record in records {
                for code in record.to_mut() {
                    *code = remap[usize::from(*code)];
                }
            }
        }
    }
    let groups = groups
        .into_iter()
        .map(|(key, records)| (key, GroupData::Fixed(records)))
        .collect();
    write_file(groups, &retained, info)
}

fn write_file(
    groups: BTreeMap<(u8, usize), GroupData<'_>>,
    retained: &[&Vec<u8>],
    info: &SourceInfo,
) -> Result<Serialized> {
    let dictionary_bytes: usize = retained.iter().map(|symbol| symbol.len() + 1).sum();
    let directory_bytes = groups
        .len()
        .checked_mul(DIR_ENTRY_LEN)
        .ok_or(Error::LimitExceeded("group directory byte length"))?;
    let mut payload_bytes = 0usize;
    let mut codec_rows = [0u64; 4];
    let mut ef_bytes = 0usize;
    for (&(codec, width), group) in &groups {
        let group_bytes = group.byte_len(width)?;
        payload_bytes = payload_bytes
            .checked_add(group_bytes)
            .ok_or(Error::LimitExceeded("payload byte length"))?;
        codec_rows[usize::from(codec)] += as_u64(group.rows(), "group row count")?;
        if matches!(group, GroupData::EliasFano { .. }) {
            ef_bytes += group_bytes;
        }
    }
    let total_bytes = HEADER_LEN
        .checked_add(dictionary_bytes)
        .and_then(|total| total.checked_add(directory_bytes))
        .and_then(|total| total.checked_add(payload_bytes))
        .ok_or(Error::LimitExceeded("serialized file byte length"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total_bytes)
        .map_err(|_| Error::LimitExceeded("serialized output allocation"))?;
    bytes.resize(HEADER_LEN, 0);
    bytes[..8].copy_from_slice(MAGIC);
    let version = if ef_bytes == 0 {
        VERSION_V2
    } else {
        VERSION_V3
    };
    bytes[8..10].copy_from_slice(&version.to_le_bytes());
    bytes[12..16].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
    bytes[16..24].copy_from_slice(&info.row_count.to_le_bytes());
    bytes[24..32].copy_from_slice(&info.raw_value_bytes.to_le_bytes());
    bytes[32..36]
        .copy_from_slice(&as_u32(info.max_value_bytes, "maximum value length")?.to_le_bytes());
    bytes[36..38].copy_from_slice(&(retained.len() as u16).to_le_bytes());
    bytes[40..44].copy_from_slice(&as_u32(groups.len(), "group count")?.to_le_bytes());
    bytes[44..48]
        .copy_from_slice(&as_u32(dictionary_bytes, "dictionary byte length")?.to_le_bytes());
    bytes[48..56].copy_from_slice(&as_u64(payload_bytes, "payload byte length")?.to_le_bytes());
    for symbol in retained {
        bytes.push(symbol.len() as u8);
        bytes.extend_from_slice(symbol);
    }
    for (&(codec, width), group) in &groups {
        bytes.extend_from_slice(&[codec, 0, 0, 0]);
        bytes.extend_from_slice(&as_u32(width, "encoded width")?.to_le_bytes());
        bytes.extend_from_slice(&as_u64(group.rows(), "group row count")?.to_le_bytes());
    }
    for group in groups.values() {
        match group {
            GroupData::Fixed(records) => {
                for record in records {
                    bytes.extend_from_slice(record);
                }
            }
            GroupData::EliasFano { bytes: block, .. } => bytes.extend_from_slice(block),
        }
    }
    debug_assert_eq!(bytes.len(), total_bytes);
    let crc = checksum(&bytes);
    bytes[56..60].copy_from_slice(&crc.to_le_bytes());
    Ok(Serialized {
        bytes,
        dictionary_symbols: retained.len(),
        dictionary_bytes,
        groups: groups.len(),
        raw_rows: codec_rows[usize::from(RAW)],
        compressed_rows: codec_rows[usize::from(PHRASES)],
        ef_mixed_rows: codec_rows[usize::from(ef::MIXED)],
        ef_power_of_two_rows: codec_rows[usize::from(ef::POWER_OF_TWO)],
        ef_bytes,
    })
}

fn as_u64(value: usize, limit: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::LimitExceeded(limit))
}

fn as_u32(value: usize, limit: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| Error::LimitExceeded(limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_pairs_use_nonoverlapping_counts() {
        let mut sequences = vec![vec![7, 7, 7, 7, 7], vec![7, 7, 8, 7, 7, 7], vec![7]];
        let mut counts = vec![0usize; 65_536];
        count_pairs(&sequences, &mut counts);
        assert_eq!(counts[(7 << 8) | 7], 4);
        assert_eq!(counts[(7 << 8) | 8], 1);
        assert_eq!(counts[(8 << 8) | 7], 1);
        assert_eq!(merge_pair(&mut sequences, 7, 7, 9), 4);
        assert_eq!(sequences, vec![vec![9, 9, 7], vec![9, 8, 9, 7], vec![7]]);
    }

    #[test]
    fn minimum_token_parse_can_beat_greedy_longest_match() {
        let dictionary = vec![
            b"a".to_vec(),
            b"b".to_vec(),
            b"c".to_vec(),
            b"d".to_vec(),
            b"e".to_vec(),
            b"abc".to_vec(),
            b"ab".to_vec(),
            b"cde".to_vec(),
        ];
        let mut encoder = Encoder::new(&dictionary);
        assert_eq!(encoder.encode_if_shorter(b"abcde"), Some(vec![6, 7]));
        assert_eq!(encoder.encode_if_shorter(b"a"), None);
    }

    #[test]
    fn no_training_and_empty_input_keep_raw_layout() {
        let options = BuildOptions {
            mode: BuildMode::Phrase,
            training_values: 0,
            ..BuildOptions::default()
        };
        let result = build(["Anne", "李", "José"], &options).unwrap();
        assert_eq!(result.stats.candidate_models, 1);
        assert_eq!(result.stats.dictionary_symbols, 0);
        assert_eq!(result.stats.raw_rows, 3);
        let empty = build(Vec::<String>::new(), &options).unwrap();
        assert_eq!(empty.bytes.len(), HEADER_LEN);
        assert_eq!(empty.stats.row_count, 0);
    }

    #[test]
    fn model_selection_is_deterministic_and_never_grows_raw_file() {
        let values: Vec<_> = (0..512)
            .map(|index| format!("repeated-surname-prefix-{index:04}"))
            .collect();
        let compact = build(&values, &BuildOptions::default()).unwrap();
        let raw = build(
            &values,
            &BuildOptions {
                mode: BuildMode::Raw,
                ..BuildOptions::default()
            },
        )
        .unwrap();
        assert!(compact.bytes.len() < raw.bytes.len());
        assert!(compact.stats.raw_rows < compact.stats.row_count);
        assert!((2..=7).contains(&compact.stats.candidate_models));
        let reversed = build(values.iter().rev(), &BuildOptions::default()).unwrap();
        assert_eq!(compact.bytes, reversed.bytes);
    }
}
