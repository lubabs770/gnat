//! A small deterministic PRNG.
//!
//! The Swift original draws from `SystemRandomNumberGenerator`, so its runs are
//! not reproducible. Seeding explicitly here costs nothing and buys a `--simtest`
//! that fails the same way twice, which matters when the pass condition is
//! statistical.

/// xoshiro128++. Fast, tiny, good enough for Poisson noise kicks.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u32; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // SplitMix64 to spread a small seed across the state; a zero state
        // would lock xoshiro at zero forever.
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        let (a, b) = (next(), next());
        Self {
            s: [a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32],
        }
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let result = self.s[0]
            .wrapping_add(self.s[3])
            .rotate_left(7)
            .wrapping_add(self.s[0]);
        let t = self.s[1] << 9;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(11);
        result
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub fn f32(&mut self) -> f32 {
        // 24 bits is the full mantissa of an f32; taking the high bits keeps
        // the best-quality end of the word.
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in `[lo, hi)`.
    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.f32() * (hi - lo)
    }

    /// Uniform integer in `[lo, hi]`.
    pub fn range_i(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(hi >= lo);
        let span = (hi - lo + 1) as u32;
        lo + (self.next_u32() % span) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_reproducible_from_a_seed() {
        let a: Vec<u32> = (0..8).map(|_| Rng::new(42).next_u32()).collect();
        assert!(a.iter().all(|&x| x == a[0]), "same seed must replay");

        let mut r1 = Rng::new(7);
        let mut r2 = Rng::new(7);
        for _ in 0..64 {
            assert_eq!(r1.next_u32(), r2.next_u32());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        assert_ne!(Rng::new(1).next_u32(), Rng::new(2).next_u32());
    }

    #[test]
    fn floats_stay_in_range_and_spread() {
        let mut r = Rng::new(99);
        let mut sum = 0.0f64;
        let n = 100_000;
        for _ in 0..n {
            let x = r.f32();
            assert!((0.0..1.0).contains(&x), "{x} out of range");
            sum += x as f64;
        }
        let mean = sum / n as f64;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean} is not uniform");
    }

    #[test]
    fn integer_range_is_inclusive_and_bounded() {
        let mut r = Rng::new(5);
        let mut lo_seen = false;
        let mut hi_seen = false;
        for _ in 0..10_000 {
            let v = r.range_i(3, 6);
            assert!((3..=6).contains(&v));
            lo_seen |= v == 3;
            hi_seen |= v == 6;
        }
        assert!(lo_seen && hi_seen, "range endpoints never drawn");
    }
}
