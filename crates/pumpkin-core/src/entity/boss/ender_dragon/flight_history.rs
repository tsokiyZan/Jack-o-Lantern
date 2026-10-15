/// Vanilla `DragonFlightHistory.Sample`.
#[derive(Clone, Copy, Default, Debug)]
pub struct Sample {
    pub y: f64,
    pub y_rot: f32,
}

/// Vanilla `DragonFlightHistory`: the last 64 heights and yaws, which the body parts trail.
#[derive(Clone, Copy, Debug)]
pub struct DragonFlightHistory {
    samples: [Sample; Self::LENGTH],
    head: i32,
}

impl Default for DragonFlightHistory {
    fn default() -> Self {
        Self {
            samples: [Sample::default(); Self::LENGTH],
            head: -1,
        }
    }
}

impl DragonFlightHistory {
    pub const LENGTH: usize = 64;
    const MASK: i32 = 63;

    pub const fn record(&mut self, y: f64, y_rot: f32) {
        let sample = Sample { y, y_rot };
        if self.head < 0 {
            self.samples = [sample; Self::LENGTH];
        }
        self.head += 1;
        if self.head == Self::LENGTH as i32 {
            self.head = 0;
        }
        self.samples[self.head as usize] = sample;
    }

    #[must_use]
    pub const fn get(&self, delay: i32) -> Sample {
        self.samples[((self.head - delay) & Self::MASK) as usize]
    }
}
