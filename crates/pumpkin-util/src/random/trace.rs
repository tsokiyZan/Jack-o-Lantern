//! A [`LegacyRand`] that records every draw, for tests that check vanilla's draw order and bounds.

use super::{RandomDeriver, RandomImpl, legacy_rand::LegacyRand};

/// One recorded call on a [`TraceRandom`], named after the vanilla `RandomSource` method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draw {
    NextInt,
    NextIntBounded(i32),
    NextLong,
    NextBoolean,
    NextFloat,
    NextDouble,
    NextGaussian,
    Skip(i32),
    Fork,
}

/// Wraps a [`LegacyRand`] and logs each call in [`Self::draws`].
pub struct TraceRandom {
    inner: LegacyRand,
    pub draws: Vec<Draw>,
}

impl TraceRandom {
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        Self {
            inner: LegacyRand::from_seed(seed),
            draws: Vec::new(),
        }
    }
}

impl RandomImpl for TraceRandom {
    fn split(&mut self) -> Self {
        self.draws.push(Draw::Fork);
        Self {
            inner: self.inner.split(),
            draws: Vec::new(),
        }
    }

    fn next_splitter(&mut self) -> RandomDeriver {
        self.draws.push(Draw::Fork);
        self.inner.next_splitter()
    }

    fn next_i32(&mut self) -> i32 {
        self.draws.push(Draw::NextInt);
        self.inner.next_i32()
    }

    fn next_bounded_i32(&mut self, bound: i32) -> i32 {
        self.draws.push(Draw::NextIntBounded(bound));
        self.inner.next_bounded_i32(bound)
    }

    fn next_i64(&mut self) -> i64 {
        self.draws.push(Draw::NextLong);
        self.inner.next_i64()
    }

    fn next_bool(&mut self) -> bool {
        self.draws.push(Draw::NextBoolean);
        self.inner.next_bool()
    }

    fn next_f32(&mut self) -> f32 {
        self.draws.push(Draw::NextFloat);
        self.inner.next_f32()
    }

    fn next_f64(&mut self) -> f64 {
        self.draws.push(Draw::NextDouble);
        self.inner.next_f64()
    }

    fn next_gaussian(&mut self) -> f64 {
        self.draws.push(Draw::NextGaussian);
        self.inner.next_gaussian()
    }

    fn skip(&mut self, count: i32) {
        self.draws.push(Draw::Skip(count));
        self.inner.skip(count);
    }
}
