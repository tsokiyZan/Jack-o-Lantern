use std::sync::atomic::Ordering;

use pumpkin_data::entity::EntityPose;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::random::RandomImpl;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;

/// Vanilla `LongJumpMidJump.TIME_OUT_DURATION`.
const TIME_OUT_DURATION: i32 = 100;

const ENTRY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 2] = [
    (types::LOOK_TARGET.id(), MemoryStatus::Registered),
    (types::LONG_JUMP_MID_JUMP.id(), MemoryStatus::ValuePresent),
];

/// Vanilla `LongJumpMidJump`: holds the long-jump pose until the mob lands, then
/// brakes, plays the landing sound and starts the cooldown.
pub struct LongJumpMidJump {
    /// Vanilla `timeBetweenLongJumps`, a `UniformInt` with both ends inclusive.
    time_between_long_jumps: (i32, i32),
    landing_sound: Sound,
}

impl LongJumpMidJump {
    #[must_use]
    pub const fn new(time_between_long_jumps: (i32, i32), landing_sound: Sound) -> Self {
        Self {
            time_between_long_jumps,
            landing_sound,
        }
    }
}

impl Behavior for LongJumpMidJump {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &ENTRY_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn max_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        !tick.mob.get_entity().on_ground.load(Ordering::Relaxed)
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        let living = &tick.mob.get_mob_entity().living_entity;
        living.discard_friction.store(true, Ordering::Relaxed);
        tick.mob.set_pose(EntityPose::LongJumping);
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        let living = &tick.mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        if entity.on_ground.load(Ordering::Relaxed) {
            let velocity = entity.velocity.load();
            entity
                .velocity
                .store(velocity.multiply(f64::from(0.1f32), 1.0, f64::from(0.1f32)));
            tick.world.play_sound_fine(
                self.landing_sound,
                SoundCategory::Neutral,
                &entity.pos.load(),
                2.0,
                1.0,
            );
        }
        living.discard_friction.store(false, Ordering::Relaxed);
        tick.mob.set_pose(EntityPose::Standing);
        tick.brain.erase(types::LONG_JUMP_MID_JUMP.id());
        let (min, max) = self.time_between_long_jumps;
        // `timeBetweenLongJumps.sample(level.getRandom())` (LongJumpMidJump.java:42), UniformInt.
        let cooldown = tick
            .world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_int_between_inclusive(min, max);
        tick.brain.set(types::LONG_JUMP_COOLDOWN_TICKS, cooldown);
    }

    fn debug_name(&self) -> &'static str {
        "LongJumpMidJump"
    }
}
