use crate::{Error, Result};

/// How a sampler maps independent uniform `u64` draws to stored ranks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SamplingMethod {
    /// Use the high half of a 128-bit product, rejecting draws whose low half
    /// falls below the threshold.
    #[default]
    MultiplyHigh,
    /// Use modulo reduction and preserve the rank sequence of [`uniform_below`].
    Modulo,
}

#[derive(Debug)]
pub(crate) struct RankSampler {
    n: u64,
    threshold: u64,
    method: SamplingMethod,
}

impl RankSampler {
    #[inline]
    pub(crate) fn new(n: u64, method: SamplingMethod) -> Self {
        // Unsigned wrapping subtraction gives (2^64 - n) for n > 0.
        let threshold = if n == 0 { 0 } else { n.wrapping_neg() % n };
        Self {
            n,
            threshold,
            method,
        }
    }

    pub(crate) fn bound(&self) -> u64 {
        self.n
    }

    #[inline]
    pub(crate) fn sample(&self, next_u64: &mut impl FnMut() -> u64) -> Result<u64> {
        if self.n == 0 {
            return Err(Error::EmptyTable);
        }
        match self.method {
            SamplingMethod::MultiplyHigh => loop {
                let product = u128::from(next_u64()) * u128::from(self.n);
                if product as u64 >= self.threshold {
                    return Ok((product >> 64) as u64);
                }
            },
            SamplingMethod::Modulo => loop {
                let draw = next_u64();
                if draw >= self.threshold {
                    return Ok(draw % self.n);
                }
            },
        }
    }
}

/// Map an independent uniform `u64` source to exactly uniform integers in `0..n`.
///
/// Values below `2^64 mod n` are rejected, leaving a multiple of `n` possible
/// source values. The caller supplies the RNG and its security properties.
/// Returns [`Error::EmptyTable`] for `n == 0` before requesting a draw. The
/// source must eventually supply an accepted draw for the sampler to return.
pub fn uniform_below<F: FnMut() -> u64>(n: u64, next_u64: &mut F) -> Result<u64> {
    RankSampler::new(n, SamplingMethod::Modulo).sample(next_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_boundary_bounds() {
        assert_eq!(SamplingMethod::default(), SamplingMethod::MultiplyHigh);
        for method in [SamplingMethod::MultiplyHigh, SamplingMethod::Modulo] {
            assert!(matches!(
                RankSampler::new(0, method).sample(&mut || panic!("empty bound drew randomness")),
                Err(Error::EmptyTable)
            ));
            let mut calls = 0;
            let rank = RankSampler::new(1, method)
                .sample(&mut || {
                    calls += 1;
                    0
                })
                .unwrap();
            assert_eq!(rank, 0);
            assert_eq!(calls, 1);
            assert_eq!(
                RankSampler::new(1 << 63, method)
                    .sample(&mut || u64::MAX)
                    .unwrap(),
                (1 << 63) - 1
            );
        }
        let expected = u64::MAX - 1;
        let rank = RankSampler::new(u64::MAX, SamplingMethod::MultiplyHigh)
            .sample(&mut || u64::MAX)
            .unwrap();
        assert_eq!(rank, expected);
        assert_eq!(uniform_below(u64::MAX, &mut || u64::MAX).unwrap(), 0);
    }

    #[test]
    fn multiply_high_rejects_by_low_product_and_returns_high_product() {
        let sampler = RankSampler::new(10, SamplingMethod::MultiplyHigh);
        let mut draws = [0, 1 << 63, 1, u64::MAX / 2, u64::MAX].into_iter();
        let mut next_u64 = || draws.next().unwrap();
        // The first two draws have a zero low product and are rejected.
        assert_eq!(sampler.sample(&mut next_u64).unwrap(), 0);
        assert_eq!(sampler.sample(&mut next_u64).unwrap(), 4);
        assert_eq!(sampler.sample(&mut next_u64).unwrap(), 9);
        assert_eq!(draws.next(), None);
    }

    #[test]
    fn exhaustive_reduced_word_proof() {
        const SHIFT: u32 = 56;
        for method in [SamplingMethod::MultiplyHigh, SamplingMethod::Modulo] {
            for bound in 1..256u64 {
                // Scaling the bound scales the rejection threshold by 2^56.
                // Multiply-high then returns the high 8-bit product unchanged.
                // Modulo scales the draw too and unscales the resulting rank.
                let sampler = RankSampler::new(bound << SHIFT, method);
                let mut counts = vec![0; bound as usize];
                for draw in 0..256u64 {
                    let first = match method {
                        SamplingMethod::MultiplyHigh => draw,
                        SamplingMethod::Modulo => draw << SHIFT,
                    };
                    let mut draws = [first, u64::MAX].into_iter();
                    let rank = sampler.sample(&mut || draws.next().unwrap()).unwrap();
                    // The second draw is accepted for every bound. Ignore a
                    // rank if the sampler had to use that second draw.
                    if draws.next().is_some() {
                        let rank = match method {
                            SamplingMethod::MultiplyHigh => rank,
                            SamplingMethod::Modulo => rank >> SHIFT,
                        };
                        counts[rank as usize] += 1;
                    }
                }
                assert!(counts.iter().all(|&count| count == 256 / bound));
            }
        }
    }
}
