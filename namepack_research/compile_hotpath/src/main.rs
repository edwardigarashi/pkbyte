#![forbid(unsafe_code)]
use pkbyte::{uniform_below, Table};
use std::hint::black_box;
use std::time::Instant;

const FILE: &[u8] = include_bytes!("../../../data/given_names_2024.npk");
const SEED: u64 = 0x5a17e6d9b0432c81;
const H: u64 = 0xcbf29ce484222325;
#[derive(Clone, Copy)]
struct Group { first: u64, end: u64, start: usize, width: usize, codec: u8 }
struct Prepared<'a> {
    payload: &'a [u8],
    bytes: [[u8; 8]; 256],
    lengths: [u8; 256],
    groups: Vec<Group>,
    hints: Vec<u8>,
    n: u64,
    max: usize,
    threshold: u64,
}
impl<'a> Prepared<'a> {
    fn from_validated(file: &'a [u8], table: &Table<'_>) -> Self {
        table.validate_all(false).unwrap();
        let dict_bytes = u32::from_le_bytes(file[44..48].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(file[36..38].try_into().unwrap()) as usize;
        let group_count = u32::from_le_bytes(file[40..44].try_into().unwrap()) as usize;
        assert!(group_count <= 256);
        let mut bytes = [[0; 8]; 256];
        let mut lengths = [0; 256];
        let mut at = 64;
        for i in 0..count {
            let len = file[at] as usize;
            at += 1;
            bytes[i][..len].copy_from_slice(&file[at..at + len]);
            lengths[i] = len as u8;
            at += len;
        }
        assert_eq!(at, 64 + dict_bytes);
        let mut groups = Vec::new();
        let mut first = 0;
        let mut start = 0;
        for _ in 0..group_count {
            let codec = file[at];
            let width = u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
            let rows = u64::from_le_bytes(file[at + 8..at + 16].try_into().unwrap());
            groups.push(Group { first, end: first + rows, start, width, codec });
            first += rows;
            start += rows as usize * width;
            at += 16;
        }
        let n = table.len();
        let hints = (0..=((n - 1) >> 5)).map(|bucket| {
            groups.partition_point(|g| g.end <= bucket << 5) as u8
        }).collect();
        Self { payload: &file[at..], bytes, lengths, groups, hints, n, max: table.max_value_bytes(), threshold: n.wrapping_neg() % n }
    }
    #[inline]
    fn get<'b, const RADIX: bool, const BULK: bool>(&self, rank: u64, out: &'b mut [u8]) -> &'b str {
        assert!(rank < self.n);
        assert!(out.len() >= self.max + 7);
        let id = if RADIX {
            let mut i = self.hints[(rank >> 5) as usize] as usize;
            while self.groups[i].end <= rank { i += 1; }
            i
        } else {
            self.groups.partition_point(|g| g.end <= rank)
        };
        let g = &self.groups[id];
        let start = g.start + (rank - g.first) as usize * g.width;
        let encoded = &self.payload[start..start + g.width];
        let len = if g.codec == 0 {
            out[..encoded.len()].copy_from_slice(encoded);
            encoded.len()
        } else {
            let mut p = 0;
            for &code in encoded {
                let len = self.lengths[code as usize] as usize;
                if BULK { out[p..p + 8].copy_from_slice(&self.bytes[code as usize]); }
                else { out[p..p + len].copy_from_slice(&self.bytes[code as usize][..len]); }
                p += len;
            }
            p
        };
        std::str::from_utf8(&out[..len]).unwrap()
    }
    #[inline]
    fn rank_mul<F: FnMut() -> u64>(&self, rng: &mut F) -> u64 {
        loop {
            let m = rng() as u128 * self.n as u128;
            if m as u64 >= self.threshold { return (m >> 64) as u64; }
        }
    }
}
#[inline]
fn mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut x = *state;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
#[inline]
fn consume(mut h: u64, bytes: &[u8]) -> u64 {
    h = (h ^ bytes.len() as u64).wrapping_mul(0x100000001b3);
    for &b in bytes { h = (h ^ b as u64).wrapping_mul(0x100000001b3); }
    h
}
fn median(a: &mut [f64]) -> f64 { a.sort_by(f64::total_cmp); a[a.len() / 2] }

fn main() {
    // Exhaust the corresponding 8-bit multiply-high algorithm for every bound.
    for bound in 1u32..256 {
        let threshold = 256 % bound;
        let mut count = vec![0u32; bound as usize];
        for x in 0u32..256 {
            let product = x * bound;
            if (product & 255) >= threshold {
                count[(product >> 8) as usize] += 1;
            }
        }
        assert!(count.iter().all(|&x| x == count[0]));
    }
    let table = Table::open(black_box(FILE)).unwrap();
    let prep = Prepared::from_validated(FILE, &table);
    let mut out = Vec::with_capacity(prep.max + 7);
    let mut fixed = vec![0u8; prep.max + 7];
    for r in 0..table.len() {
        let expected = table.get_into(r, &mut out).unwrap();
        assert_eq!(expected, prep.get::<false, false>(r, &mut fixed));
        assert_eq!(expected, prep.get::<false, true>(r, &mut fixed));
        assert_eq!(expected, prep.get::<true, true>(r, &mut fixed));
    }
    let methods = ["original_mod", "prepared_variable_mod", "prepared_bulk_mod", "prepared_bulk_radix_mod", "original_mul", "prepared_bulk_radix_mul"];
    let iters = 1_000_000usize;
    let rounds = 11usize;
    let mut times = vec![vec![]; methods.len()];
    let mut checks = [0u64; 2];
    for round in 0..rounds + 1 {
        for index in 0..methods.len() {
            let method = (index + round) % methods.len();
            let mut seed = black_box(SEED);
            let mut random = || mix(&mut seed);
            let mut hash = H;
            let timer = Instant::now();
            match method {
                0 => for _ in 0..iters { hash = consume(hash, table.sample_into(&mut random, &mut out).unwrap().as_bytes()); },
                1 => for _ in 0..iters { let r = uniform_below(prep.n, &mut random).unwrap(); hash = consume(hash, prep.get::<false, false>(r, &mut fixed).as_bytes()); },
                2 => for _ in 0..iters { let r = uniform_below(prep.n, &mut random).unwrap(); hash = consume(hash, prep.get::<false, true>(r, &mut fixed).as_bytes()); },
                3 => for _ in 0..iters { let r = uniform_below(prep.n, &mut random).unwrap(); hash = consume(hash, prep.get::<true, true>(r, &mut fixed).as_bytes()); },
                4 => for _ in 0..iters { let r = prep.rank_mul(&mut random); hash = consume(hash, table.get_into(r, &mut out).unwrap().as_bytes()); },
                5 => for _ in 0..iters { let r = prep.rank_mul(&mut random); hash = consume(hash, prep.get::<true, true>(r, &mut fixed).as_bytes()); },
                _ => unreachable!(),
            }
            let ns = timer.elapsed().as_secs_f64() * 1e9 / iters as f64;
            black_box(hash);
            let check = if method < 4 { 0 } else { 1 };
            if checks[check] == 0 { checks[check] = hash; }
            assert_eq!(checks[check], hash);
            if round > 0 { times[method].push(ns); }
        }
    }
    println!("{{\"names\":{},\"source_bytes\":{},\"iterations\":{iters},\"rounds\":{rounds},\"warmup_rounds\":1,\"symbol_bytes\":{},\"groups_bytes\":{},\"hint_bytes\":{},\"scratch_bytes\":{},\"checksums\":[\"{:016x}\",\"{:016x}\"],\"results\":[", prep.n, FILE.len(), std::mem::size_of_val(&prep.bytes) + std::mem::size_of_val(&prep.lengths), prep.groups.len() * std::mem::size_of::<Group>(), prep.hints.len(), fixed.len(), checks[0], checks[1]);
    for (i, name) in methods.iter().enumerate() {
        let median = median(&mut times[i]);
        println!("{{\"method\":\"{name}\",\"median_ns\":{median},\"all_ns\":{:?}}}{}", times[i], if i + 1 == methods.len() { "" } else { "," });
    }
    println!("]}}");
}
