use std::sync::Arc;

use pumpkin_util::math::wrap_degrees;
use pumpkin_util::random::RandomImpl;

use super::super::BrainTick;
use super::super::memory::position_tracker::{BlockPosTracker, PositionTracker};
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;
use crate::entity::Entity;

/// Vanilla `RandomLookAround`: looks at a random spot ahead, then waits out a gaze
/// cooldown before the next one.
pub struct RandomLookAround {
    interval: (i32, i32),
    max_yaw: f32,
    min_pitch: f32,
    pitch_range: f32,
    conditions: [(MemoryModuleId, MemoryStatus); 2],
}

impl RandomLookAround {
    /// `interval` is vanilla's `UniformInt` bounds, both inclusive.
    #[must_use]
    pub const fn new(interval: (i32, i32), max_yaw: f32, min_pitch: f32, max_pitch: f32) -> Self {
        Self {
            interval,
            max_yaw,
            min_pitch,
            pitch_range: max_pitch - min_pitch,
            conditions: [
                (types::LOOK_TARGET.id(), MemoryStatus::ValueAbsent),
                (types::GAZE_COOLDOWN_TICKS.id(), MemoryStatus::ValueAbsent),
            ],
        }
    }
}

impl Behavior for RandomLookAround {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        let entity = tick.mob.get_entity();
        let (pitch_roll, yaw_roll) = {
            let mut random = entity.random();
            (random.next_f32(), random.next_f32())
        };
        let pitch = (pitch_roll * self.pitch_range + self.min_pitch).clamp(-90.0, 90.0);
        let rotation =
            wrap_degrees(entity.yaw.load() + 2.0 * yaw_roll * self.max_yaw - self.max_yaw);
        let look_at = tick.mob.get_eye_pos() + Entity::direction_from_rotation(pitch, rotation);
        tick.brain.set(
            types::LOOK_TARGET,
            Arc::new(BlockPosTracker::from_vec(look_at)) as Arc<dyn PositionTracker>,
        );
        let cooldown = entity
            .random()
            .next_int_between_inclusive(self.interval.0, self.interval.1);
        tick.brain.set(types::GAZE_COOLDOWN_TICKS, cooldown);
    }

    fn debug_name(&self) -> &'static str {
        "RandomLookAround"
    }
}
