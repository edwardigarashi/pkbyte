//! Bounded mixed-radix Elias–Fano blocks for equal-width values.

use crate::{Error, Result};

pub(crate) const MIXED: u8 = 2;
pub(crate) const POWER_OF_TWO: u8 = 3;
pub(crate) const MAX_WIDTH: usize = 64;

const HEADER_BYTES: usize = 24;
const CHECKPOINT_ROWS: u64 = 64;
const MAX_SELECT_SPAN: u64 = 2_048;

#[derive(Debug)]
pub(crate) struct Block {
    base: u64,
    width: usize,
    count: u64,
    low_width: u32,
    codec: u8,
    // Section boundaries relative to the same immutable payload checked at open.
    sections: [usize; 5],
}

struct Sections<'a> {
    alphabets: &'a [u8],
    lows: &'a [u8],
    highs: &'a [u8],
    checkpoints: &'a [u8],
}

fn invalid() -> Error {
    Error::InvalidFormat("invalid Elias–Fano block")
}

fn read<const N: usize>(input: &[u8], at: usize) -> Option<[u8; N]> {
    input.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn read_word(input: &[u8], word: u64) -> Option<u64> {
    let at = usize::try_from(word).ok()?.checked_mul(8)?;
    Some(u64::from_le_bytes(read(input, at)?))
}

fn word_bytes(bits: u64) -> Option<usize> {
    usize::try_from(bits.div_ceil(64)).ok()?.checked_mul(8)
}

fn read_alphabet(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let len = usize::from(u16::from_le_bytes(read(input, 0)?));
    if !(1..=256).contains(&len) {
        return None;
    }
    Some((input.get(2..2 + len)?, input.get(2 + len..)?))
}

fn radix(codec: u8, len: usize) -> Option<u64> {
    match codec {
        MIXED => Some(len as u64),
        POWER_OF_TWO => Some(len.next_power_of_two() as u64),
        _ => None,
    }
}

fn low_width(last_delta: u64, count: u64) -> Option<u32> {
    Some(
        last_delta
            .checked_add(1)?
            .checked_div(count)?
            .max(1)
            .ilog2(),
    )
}

fn read_low(lows: &[u8], width: u32, rank: u64) -> Option<u64> {
    if width == 0 {
        return Some(0);
    }
    let bit = rank.checked_mul(u64::from(width))?;
    let index = bit / 64;
    let shift = (bit % 64) as u32;
    let mut value = read_word(lows, index)? >> shift;
    if shift + width > 64 {
        value |= read_word(lows, index + 1)? << (64 - shift);
    }
    Some(value & ((1u64 << width) - 1))
}

impl Block {
    pub(crate) fn open(
        payload: &[u8],
        offset: usize,
        width: usize,
        count: u64,
        codec: u8,
    ) -> Result<(Self, usize)> {
        if !(1..=MAX_WIDTH).contains(&width) || count == 0 || radix(codec, 1).is_none() {
            return Err(invalid());
        }
        let input = payload.get(offset..).ok_or_else(invalid)?;
        let base = u64::from_le_bytes(read(input, 0).ok_or_else(invalid)?);
        let size = usize::try_from(u32::from_le_bytes(read(input, 8).ok_or_else(invalid)?))
            .map_err(|_| invalid())?;
        let alphabet_bytes =
            usize::try_from(u32::from_le_bytes(read(input, 12).ok_or_else(invalid)?))
                .map_err(|_| invalid())?;
        let high_bits = u64::from(u32::from_le_bytes(read(input, 16).ok_or_else(invalid)?));
        let [low_width, reserved @ ..] = read::<4>(input, 20).ok_or_else(invalid)?;
        let low_width = u32::from(low_width);
        if reserved != [0; 3]
            || low_width >= 64
            || high_bits < count
            || high_bits > count.saturating_mul(3)
        {
            return Err(invalid());
        }

        let low_bits = count
            .checked_mul(u64::from(low_width))
            .ok_or_else(invalid)?;
        let low_bytes = word_bytes(low_bits).ok_or_else(invalid)?;
        let high_bytes = word_bytes(high_bits).ok_or_else(invalid)?;
        let checkpoint_bytes = usize::try_from(count.div_ceil(CHECKPOINT_ROWS))
            .ok()
            .and_then(|entries| entries.checked_mul(4))
            .ok_or_else(invalid)?;
        let alphabet_end = HEADER_BYTES
            .checked_add(alphabet_bytes)
            .ok_or_else(invalid)?;
        let low_end = alphabet_end.checked_add(low_bytes).ok_or_else(invalid)?;
        let high_end = low_end.checked_add(high_bytes).ok_or_else(invalid)?;
        if high_end.checked_add(checkpoint_bytes) != Some(size) || size > input.len() {
            return Err(invalid());
        }

        let block = Self {
            base,
            width,
            count,
            low_width,
            codec,
            sections: [HEADER_BYTES, alphabet_end, low_end, high_end, size].map(|end| offset + end),
        };
        let data = block.data(payload);
        let mut alphabets = data.alphabets;
        let mut universe = 1u64;
        for _ in 0..width {
            let (alphabet, rest) = read_alphabet(alphabets).ok_or_else(invalid)?;
            if alphabet.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(invalid());
            }
            universe = universe
                .checked_mul(radix(codec, alphabet.len()).ok_or_else(invalid)?)
                .ok_or_else(invalid)?;
            alphabets = rest;
        }
        if !alphabets.is_empty() {
            return Err(invalid());
        }

        let unpadded = low_bits % 64;
        if unpadded != 0
            && read_word(data.lows, low_bits / 64).ok_or_else(invalid)? >> unpadded != 0
        {
            return Err(invalid());
        }
        block.validate_highs(&data, high_bits, universe)?;
        Ok((block, size))
    }

    fn data<'a>(&self, payload: &'a [u8]) -> Sections<'a> {
        let [alphabets, lows, highs, checkpoints, end] = self.sections;
        Sections {
            alphabets: &payload[alphabets..lows],
            lows: &payload[lows..highs],
            highs: &payload[highs..checkpoints],
            checkpoints: &payload[checkpoints..end],
        }
    }

    fn checkpoint(checkpoints: &[u8], rank: u64) -> Option<u64> {
        let index = usize::try_from(rank / CHECKPOINT_ROWS)
            .ok()?
            .checked_mul(4)?;
        Some(u64::from(u32::from_le_bytes(read(checkpoints, index)?)))
    }

    fn delta(&self, lows: &[u8], rank: u64, bit: u64) -> Option<u64> {
        let high = bit.checked_sub(rank)?;
        // A checked shift only checks the shift amount, so prove that the
        // high part also fits once shifted.
        if high > u64::MAX >> self.low_width {
            return None;
        }
        Some((high << self.low_width) | read_low(lows, self.low_width, rank)?)
    }

    fn validate_highs(&self, data: &Sections<'_>, high_bits: u64, universe: u64) -> Result<()> {
        let mut rank = 0u64;
        let mut checkpoint = 0u64;
        let mut last_bit = 0u64;
        let mut last_delta = 0u64;
        for word_index in 0..high_bits.div_ceil(64) {
            let mut word = read_word(data.highs, word_index).ok_or_else(invalid)?;
            while word != 0 {
                let bit = word_index * 64 + u64::from(word.trailing_zeros());
                if bit >= high_bits || rank >= self.count {
                    return Err(invalid());
                }
                if rank % CHECKPOINT_ROWS == 0 {
                    if Self::checkpoint(data.checkpoints, rank) != Some(bit) {
                        return Err(invalid());
                    }
                    checkpoint = bit;
                } else if bit - checkpoint > MAX_SELECT_SPAN {
                    return Err(invalid());
                }
                let delta = self.delta(data.lows, rank, bit).ok_or_else(invalid)?;
                let value = self.base.checked_add(delta).ok_or_else(invalid)?;
                if value >= universe
                    || (rank == 0 && delta != 0)
                    || (rank != 0 && delta <= last_delta)
                {
                    return Err(invalid());
                }
                last_bit = bit;
                last_delta = delta;
                rank += 1;
                word &= word - 1;
            }
        }
        if rank != self.count
            || last_bit + 1 != high_bits
            || low_width(last_delta, self.count) != Some(self.low_width)
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn select(data: &Sections<'_>, rank: u64) -> Option<u64> {
        let start = Self::checkpoint(data.checkpoints, rank)?;
        let first_word = start / 64;
        let mut skip = (rank % CHECKPOINT_ROWS) as u32;
        let mut mask = u64::MAX << (start % 64);
        for index in first_word..=first_word + MAX_SELECT_SPAN / 64 {
            let mut word = read_word(data.highs, index)? & mask;
            let ones = word.count_ones();
            if skip < ones {
                for _ in 0..skip {
                    word &= word - 1;
                }
                return Some(index * 64 + u64::from(word.trailing_zeros()));
            }
            skip -= ones;
            mask = u64::MAX;
        }
        None
    }

    pub(crate) fn decode(&self, payload: &[u8], local_rank: u64, output: &mut [u8]) -> Result<()> {
        if local_rank >= self.count {
            return Err(Error::OutOfBounds {
                index: local_rank,
                len: self.count,
            });
        }
        let provided = output.len();
        let output = output.get_mut(..self.width).ok_or(Error::OutputTooSmall {
            required: self.width,
            provided,
        })?;
        let data = self.data(payload);
        let bit = Self::select(&data, local_rank).ok_or_else(invalid)?;
        let delta = self.delta(data.lows, local_rank, bit).ok_or_else(invalid)?;
        let mut value = self.base.checked_add(delta).ok_or_else(invalid)?;
        let mut alphabets = data.alphabets;
        for byte in output.iter_mut().rev() {
            let (alphabet, rest) = read_alphabet(alphabets).ok_or_else(invalid)?;
            let radix = radix(self.codec, alphabet.len()).ok_or_else(invalid)?;
            let digit = if self.codec == POWER_OF_TWO {
                let digit = value & (radix - 1);
                value >>= radix.trailing_zeros();
                digit
            } else {
                let digit = value % radix;
                value /= radix;
                digit
            };
            *byte = *alphabet.get(digit as usize).ok_or_else(invalid)?;
            alphabets = rest;
        }
        if value != 0 {
            return Err(invalid());
        }
        Ok(())
    }
}

pub(crate) fn encode(values: &[&[u8]], codec: u8) -> Option<Vec<u8>> {
    let width = values.first()?.len();
    if !(1..=MAX_WIDTH).contains(&width) || values.iter().any(|value| value.len() != width) {
        return None;
    }
    let count = u64::try_from(values.len()).ok()?;
    let mut alphabet_bytes = Vec::new();
    let mut alphabets = Vec::with_capacity(width);
    let mut universe = 1u64;
    for position in (0..width).rev() {
        let mut present = [false; 256];
        for value in values {
            present[usize::from(value[position])] = true;
        }
        let mut digits = [0u8; 256];
        let mut alphabet = Vec::new();
        for (byte, present) in present.into_iter().enumerate() {
            if present {
                digits[byte] = alphabet.len() as u8;
                alphabet.push(byte as u8);
            }
        }
        let radix = radix(codec, alphabet.len())?;
        universe = universe.checked_mul(radix)?;
        alphabet_bytes.extend_from_slice(&(alphabet.len() as u16).to_le_bytes());
        alphabet_bytes.extend_from_slice(&alphabet);
        alphabets.push((digits, radix));
    }

    let mut integers = Vec::new();
    integers.try_reserve_exact(values.len()).ok()?;
    for value in values {
        let mut integer = 0u64;
        for (&byte, (digits, radix)) in value.iter().zip(alphabets.iter().rev()) {
            integer = integer
                .checked_mul(*radix)?
                .checked_add(u64::from(digits[usize::from(byte)]))?;
        }
        if integers.last().is_some_and(|&previous| previous >= integer) {
            return None;
        }
        integers.push(integer);
    }
    let base = integers[0];
    let last_delta = integers.last()? - base;
    let low_width = low_width(last_delta, count)?;
    let high_bits = (last_delta >> low_width).checked_add(count)?;
    if high_bits > count.saturating_mul(3) {
        return None;
    }
    let high_bits_field = u32::try_from(high_bits).ok()?;
    let low_bits = count.checked_mul(u64::from(low_width))?;
    let low_bytes = word_bytes(low_bits)?;
    let high_bytes = word_bytes(high_bits)?;
    let checkpoint_bytes = usize::try_from(count.div_ceil(CHECKPOINT_ROWS))
        .ok()?
        .checked_mul(4)?;
    let size = HEADER_BYTES
        .checked_add(alphabet_bytes.len())?
        .checked_add(low_bytes)?
        .checked_add(high_bytes)?
        .checked_add(checkpoint_bytes)?;
    let size_field = u32::try_from(size).ok()?;

    let word_count = low_bytes.checked_add(high_bytes)? / 8;
    let mut words = Vec::new();
    words.try_reserve_exact(word_count).ok()?;
    words.resize(word_count, 0u64);
    let (lows, highs) = words.split_at_mut(low_bytes / 8);
    let mut checkpoints = Vec::new();
    checkpoints.try_reserve_exact(checkpoint_bytes).ok()?;
    let mut checkpoint = 0;
    let low_mask = (1u64 << low_width) - 1;
    for (rank, integer) in integers.into_iter().enumerate() {
        let rank = rank as u64;
        let delta = integer - base;
        let select_bit = (delta >> low_width) + rank;
        if rank % CHECKPOINT_ROWS == 0 {
            checkpoint = select_bit;
            checkpoints.extend_from_slice(&(select_bit as u32).to_le_bytes());
        } else if select_bit - checkpoint > MAX_SELECT_SPAN {
            return None;
        }
        highs[(select_bit / 64) as usize] |= 1u64 << (select_bit % 64);
        if low_width > 0 {
            let bit = rank * u64::from(low_width);
            let index = (bit / 64) as usize;
            let shift = (bit % 64) as u32;
            let low = delta & low_mask;
            lows[index] |= low << shift;
            if shift + low_width > 64 {
                lows[index + 1] |= low >> (64 - shift);
            }
        }
    }

    let mut output = Vec::new();
    output.try_reserve_exact(size).ok()?;
    output.extend_from_slice(&base.to_le_bytes());
    output.extend_from_slice(&size_field.to_le_bytes());
    output.extend_from_slice(&(alphabet_bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(&high_bits_field.to_le_bytes());
    output.extend_from_slice(&[low_width as u8, 0, 0, 0]);
    output.extend_from_slice(&alphabet_bytes);
    for word in words {
        output.extend_from_slice(&word.to_le_bytes());
    }
    output.extend_from_slice(&checkpoints);
    debug_assert_eq!(output.len(), size);
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_block_codecs_roundtrip_across_words_and_checkpoints() {
        let values: Vec<_> = (1..1500)
            .filter(|value| value % 7 < 3)
            .map(|value| format!("{value:04}"))
            .collect();
        let values: Vec<_> = values.iter().map(|value| value.as_bytes()).collect();
        for codec in [MIXED, POWER_OF_TWO] {
            let block_bytes = encode(&values, codec).unwrap();
            let block_len = block_bytes.len();
            let mut encoded = vec![0xff; 3];
            encoded.extend_from_slice(&block_bytes);
            encoded.extend_from_slice(b"next section");
            let (block, size) = Block::open(&encoded, 3, 4, values.len() as u64, codec).unwrap();
            assert_eq!(size, block_len);
            assert!(block.base > 0);
            assert!(block.low_width > 0);
            for (rank, expected) in values.iter().enumerate() {
                let mut output = [0xff; 5];
                block.decode(&encoded, rank as u64, &mut output).unwrap();
                assert_eq!(&output[..4], *expected);
                assert_eq!(output[4], 0xff);
            }
        }
    }

    #[test]
    fn overflowing_radix_and_incompatible_values_fall_back() {
        let first = [b'a'; MAX_WIDTH];
        let last = [b'b'; MAX_WIDTH];
        for codec in [MIXED, POWER_OF_TWO] {
            assert!(encode(&[&first, &last], codec).is_none());
            assert!(encode(&[b""], codec).is_none());
            assert!(encode(&[b"xx", b"xxx"], codec).is_none());
            assert!(encode(&[b"z", b"a"], codec).is_none());
            assert!(encode(&[b"z", b"z"], codec).is_none());
        }
    }

    #[test]
    fn structure_checks_padding_and_selected_decode_checks_unused_digit() {
        let encoded = encode(&[b"A1", b"B0", b"C2"], POWER_OF_TWO).unwrap();
        let low_start = HEADER_BYTES + 10;
        for truncated in 0..encoded.len() {
            assert!(Block::open(&encoded[..truncated], 0, 2, 3, POWER_OF_TWO).is_err());
        }

        let mut bad_checkpoint = encoded.clone();
        let checkpoint_start = bad_checkpoint.len() - 4;
        bad_checkpoint[checkpoint_start] = 1;
        assert!(Block::open(&bad_checkpoint, 0, 2, 3, POWER_OF_TWO).is_err());

        let mut bad_padding = encoded.clone();
        bad_padding[low_start] |= 0x80;
        assert!(Block::open(&bad_padding, 0, 2, 3, POWER_OF_TWO).is_err());

        let mut unused_digit = encoded;
        // Integer 4 becomes 3 when the low bit of delta 3 is cleared. It
        // still lies in the rounded radix universe but digit 3 is unused.
        unused_digit[low_start] &= !(1 << 1);
        let (block, _) = Block::open(&unused_digit, 0, 2, 3, POWER_OF_TWO).unwrap();
        let mut output = [0; 2];
        assert!(block.decode(&unused_digit, 1, &mut output).is_err());
        block.decode(&unused_digit, 2, &mut output).unwrap();
        assert_eq!(&output, b"C2");
    }
}
