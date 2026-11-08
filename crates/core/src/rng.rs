//! Small deterministic random number generator.
//!
//! primitive uses Go's `math/rand`. We don't need to reproduce its exact
//! sequence, only to be deterministic, so this is Xoshiro256++ seeded through
//! SplitMix64 (the seeding scheme its authors recommend), plus a Box–Muller
//! normal distribution for the triangle mutations.
//!
//! Every hill-climb search gets its own generator derived from
//! `(seed, step, search)`. That is what makes the output identical no matter
//! how many threads run the searches, or in which order they finish.

/// Xoshiro256++ pseudo-random number generator.
#[derive(Debug, Clone)]
pub(crate) struct Rng {
    state: [u64; 4],
}

/// Advances a SplitMix64 state and returns the next output.
///
/// SplitMix64 turns any 64-bit value (even 0) into well-mixed output, which is
/// what Xoshiro needs for its initial state.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Rng {
    /// Creates a generator from a single 64-bit seed.
    pub(crate) fn new(seed: u64) -> Rng {
        let mut sm = seed;
        let state = [
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
        ];
        Rng { state }
    }

    /// Creates the generator for one hill-climb search.
    ///
    /// The user seed, the step number and the search number are mixed one
    /// after the other, so every `(seed, step, search)` triple gets an
    /// unrelated stream.
    pub(crate) fn for_search(seed: u64, step: u64, search: u64) -> Rng {
        let mut sm = seed;
        let mut key = splitmix64(&mut sm);
        sm = key ^ step;
        key = splitmix64(&mut sm);
        sm = key ^ search;
        key = splitmix64(&mut sm);
        Rng::new(key)
    }

    /// Returns the next 64 random bits.
    pub(crate) fn next_u64(&mut self) -> u64 {
        let s = &mut self.state;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Returns a uniform integer in `0..n`.
    ///
    /// Uses a plain modulo. Its bias is about `n / 2^64`, which is far too
    /// small to matter for the image-sized ranges used here.
    ///
    /// # Panics
    /// Panics if `n` is 0.
    pub(crate) fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0, "Rng::below called with n = 0");
        (self.next_u64() % u64::from(n)) as u32
    }

    /// Returns a uniform float in `[0, 1)` built from the top 53 random bits.
    pub(crate) fn next_f64(&mut self) -> f64 {
        const SCALE: f64 = 1.0 / (1u64 << 53) as f64;
        (self.next_u64() >> 11) as f64 * SCALE
    }

    /// Returns a sample from the standard normal distribution (mean 0, standard
    /// deviation 1), like Go's `rand.NormFloat64`.
    ///
    /// Box–Muller transform. `u1` is taken from `(0, 1]` so `ln` never sees 0.
    pub(crate) fn normal(&mut self) -> f64 {
        let u1 = 1.0 - self.next_f64();
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_search_coordinates_give_different_sequences() {
        let first: Vec<u64> = {
            let mut rng = Rng::for_search(1, 0, 0);
            (0..8).map(|_| rng.next_u64()).collect()
        };
        for (step, search) in [(0, 1), (1, 0), (1, 1), (7, 3)] {
            let mut rng = Rng::for_search(1, step, search);
            let other: Vec<u64> = (0..8).map(|_| rng.next_u64()).collect();
            assert_ne!(first, other, "step {step} search {search}");
        }
        // And the same triple always reproduces the same stream.
        let mut again = Rng::for_search(1, 0, 0);
        let repeat: Vec<u64> = (0..8).map(|_| again.next_u64()).collect();
        assert_eq!(first, repeat);
    }

    #[test]
    fn below_stays_in_range() {
        let mut rng = Rng::new(7);
        for n in [1, 2, 3, 31, 256, 1000] {
            for _ in 0..1000 {
                assert!(rng.below(n) < n);
            }
        }
    }

    #[test]
    fn next_f64_stays_in_unit_interval() {
        let mut rng = Rng::new(9);
        for _ in 0..10_000 {
            let x = rng.next_f64();
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn normal_has_roughly_zero_mean_and_unit_variance() {
        let mut rng = Rng::new(123);
        let n = 10_000;
        let samples: Vec<f64> = (0..n).map(|_| rng.normal()).collect();
        let mean = samples.iter().sum::<f64>() / n as f64;
        let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        assert!(mean.abs() < 0.05, "mean {mean}");
        assert!((variance - 1.0).abs() < 0.05, "variance {variance}");
    }
}
