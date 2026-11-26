//! Experimental reader for producer-verified research artifacts only.
//! This is NOT a hardened parser for untrusted input.
use std::fs;

fn u16le(b: &[u8], p: usize) -> u16 { u16::from_le_bytes(b[p..p+2].try_into().unwrap()) }
fn u32le(b: &[u8], p: usize) -> u32 { u32::from_le_bytes(b[p..p+4].try_into().unwrap()) }
fn u64le(b: &[u8], p: usize) -> u64 { u64::from_le_bytes(b[p..p+8].try_into().unwrap()) }

struct Alphabet { bytes: Vec<u8>, radix: u64, reciprocal: u64, shift: u32 }
struct Group {
    first: u64, end: u64, base: u64, low: u32, qlog: u32,
    alphabets: Vec<Alphabet>, lows: Vec<u64>, highs: Vec<u64>, checkpoints: Vec<u32>,
    shifted: bool,
}
pub struct Ef { groups: Vec<Group>, n: u64, hints: Vec<u8>, pub file_bytes: usize }

const fn select8_table() -> [[u8; 8]; 256] {
    let mut result = [[0; 8]; 256];
    let mut value = 1;
    while value < 256 {
        let mut bit = 0;
        let mut ordinal = 0;
        while bit < 8 {
            if value & (1 << bit) != 0 { result[value][ordinal] = bit as u8; ordinal += 1; }
            bit += 1;
        }
        value += 1;
    }
    result
}
static SELECT8: [[u8; 8]; 256] = select8_table();

#[inline]
fn select64(mut word: u64, mut nth: u32) -> usize {
    let mut offset = 0;
    let c = (word as u32).count_ones();
    if nth >= c { nth -= c; word >>= 32; offset += 32; }
    let c = (word as u16).count_ones();
    if nth >= c { nth -= c; word >>= 16; offset += 16; }
    let c = (word as u8).count_ones();
    if nth >= c { nth -= c; word >>= 8; offset += 8; }
    offset + SELECT8[word as u8 as usize][nth as usize] as usize
}

impl Ef {
    pub fn load(path: &str) -> Self {
        let data = fs::read(path).unwrap();
        assert_eq!(&data[..8], b"NAMESEF\0");
        assert_eq!(u32le(&data, 8), 1);
        let count = u32le(&data, 12) as usize;
        let n = u64le(&data, 16);
        assert!(count <= 256 && n > 0);
        let mut pos = 64 + 48 * count;
        let mut first = 0;
        let mut groups = Vec::new();
        for g in 0..count {
            let d = 64 + 48 * g;
            let length = u16le(&data, d) as usize;
            let low = data[d+2] as u32;
            let qlog = data[d+3] as u32;
            let rows = u32le(&data, d+4) as u64;
            let base = u64le(&data, d+8);
            let abytes = u32le(&data, d+16) as usize;
            let lbytes = u32le(&data, d+20) as usize;
            let hbytes = u32le(&data, d+24) as usize;
            let cpcount = u32le(&data, d+28) as usize;
            let flags = u32le(&data, d+36);
            assert!(flags <= 1 && low < 64);
            let end_alph = pos + abytes;
            let mut alphabets = Vec::new();
            for _ in 0..length {
                let k = u16le(&data, pos) as usize;
                pos += 2;
                let bytes = data[pos..pos+k].to_vec();
                pos += k;
                let shift = (k as u64).next_power_of_two().trailing_zeros();
                let radix = if flags == 1 { 1u64 << shift } else { k as u64 };
                let reciprocal = if radix == 1 { 0 } else { ((1u128 << 64) / radix as u128) as u64 };
                alphabets.push(Alphabet { bytes, radix, reciprocal, shift });
            }
            assert_eq!(pos, end_alph);
            let lows = (0..lbytes/8).map(|i| u64le(&data, pos+8*i)).collect(); pos += lbytes;
            let highs = (0..hbytes/8).map(|i| u64le(&data, pos+8*i)).collect(); pos += hbytes;
            let checkpoints = (0..cpcount).map(|i| u32le(&data, pos+4*i)).collect(); pos += 4*cpcount;
            groups.push(Group { first, end: first + rows, base, low, qlog, alphabets, lows, highs, checkpoints, shifted: flags == 1 });
            first += rows;
        }
        assert_eq!(pos, data.len()); assert_eq!(first, n);
        let hints = (0..=((n-1)>>5)).map(|i| groups.partition_point(|g| g.end <= i << 5) as u8).collect();
        Self { groups, n, hints, file_bytes: data.len() }
    }
    pub fn len(&self) -> u64 { self.n }
    #[inline]
    pub fn get<'a, const RECIP: bool>(&self, rank: u64, out: &'a mut [u8]) -> &'a str {
        assert!(rank < self.n);
        let mut gi = self.hints[(rank >> 5) as usize] as usize;
        while self.groups[gi].end <= rank { gi += 1; }
        let g = &self.groups[gi];
        let rank = rank - g.first;
        let bit = g.checkpoints[(rank >> g.qlog) as usize] as usize;
        let mut skip = (rank & ((1 << g.qlog) - 1)) as u32;
        let mut word_id = bit >> 6;
        let mut word = g.highs[word_id] & (u64::MAX << (bit & 63));
        loop {
            let count = word.count_ones();
            if skip < count { break; }
            skip -= count; word_id += 1; word = g.highs[word_id];
        }
        let selected = (word_id * 64 + select64(word, skip)) as u64;
        let p = rank * g.low as u64;
        let low = if g.low == 0 { 0 } else {
            let wi = (p >> 6) as usize;
            let shift = (p & 63) as u32;
            let mut x = g.lows[wi] >> shift;
            if shift + g.low > 64 { x |= g.lows[wi+1] << (64-shift); }
            x & ((1u64 << g.low)-1)
        };
        let mut value = g.base + ((selected - rank) << g.low) + low;
        for (i, alphabet) in g.alphabets.iter().enumerate().rev() {
            let digit;
            if g.shifted {
                digit = value & (alphabet.radix - 1);
                value >>= alphabet.shift;
            } else if RECIP && alphabet.radix != 1 {
                let mut quotient = ((value as u128 * alphabet.reciprocal as u128) >> 64) as u64;
                let mut rem = value - quotient * alphabet.radix;
                if rem >= alphabet.radix { quotient += 1; rem -= alphabet.radix; }
                digit = rem; value = quotient;
            } else {
                digit = value % alphabet.radix; value /= alphabet.radix;
            }
            out[i] = alphabet.bytes[digit as usize];
        }
        assert_eq!(value, 0);
        std::str::from_utf8(&out[..g.alphabets.len()]).unwrap()
    }
}
