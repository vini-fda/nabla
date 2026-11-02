//! A small seeded random number generator, for tests and for the network simulator, where a
//! failing seed must reproduce the same run on every machine.

/// `SplitMix64`: a 64-bit generator that passes the usual statistical tests, from a single `u64`
/// of state. Its sequence is a function of the seed and nothing else.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// An independent stream for `stream`, from the same seed: runs that must not disturb each
    /// other's randomness, such as the links of different connections, each take their own.
    #[must_use]
    pub fn fork(seed: u64, stream: u64) -> Self {
        let mut rng = Self::new(seed ^ stream.wrapping_mul(0xD1B5_4A32_D192_ED03));
        // Mix, so that neighboring streams do not start alike.
        rng.next_u64();
        rng
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// Uniform in `0..n`, for `n > 0`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// `true` with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Any bit pattern of an `f64` whatever: NaN of every payload, the infinities, subnormals.
    pub fn any_bits(&mut self) -> f64 {
        f64::from_bits(self.next_u64())
    }

    /// A number a hostile client might send: half the time any bit pattern, otherwise one of the
    /// numbers that break code (NaN, the infinities, zeros of both signs, the extremes), or a
    /// number near the range a ship works in.
    pub fn wild(&mut self) -> f64 {
        const SPECIAL: [f64; 14] = [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
            1.0,
            -1.0,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE,
            f64::EPSILON,
            1e300,
            -1e-300,
            std::f64::consts::PI,
        ];
        match self.below(4) {
            0 => self.any_bits(),
            1 => SPECIAL[self.below(SPECIAL.len() as u64) as usize],
            2 => self.range(-100.0, 100.0),
            _ => self.range(-3.0, 3.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_gives_one_sequence_and_streams_differ() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        assert_eq!(
            (0..8).map(|_| a.next_u64()).collect::<Vec<_>>(),
            (0..8).map(|_| b.next_u64()).collect::<Vec<_>>(),
            "same seed"
        );
        assert_ne!(
            Rng::fork(7, 0).next_u64(),
            Rng::fork(7, 1).next_u64(),
            "two streams"
        );
    }

    #[test]
    fn unit_numbers_stay_in_range_and_wild_ones_reach_the_edge_cases() {
        let mut rng = Rng::new(1);
        let mut seen = [false; 3];
        for _ in 0..10_000 {
            let u = rng.unit();
            assert!((0.0..1.0).contains(&u), "{u}");
            let w = rng.wild();
            seen[0] |= w.is_nan();
            seen[1] |= w.is_infinite();
            seen[2] |= w.is_finite() && w != 0.0;
        }
        assert!(seen.iter().all(|&s| s), "NaN, infinity and a number");
    }
}
