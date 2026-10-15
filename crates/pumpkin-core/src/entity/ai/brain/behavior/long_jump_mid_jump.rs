use std::sync::atomic::Ordering::{Relaxed, SeqCst};

use pumpkin_data::entity::EntityPose;
use pumpkin_data::sound::{Sound, SoundCategory};

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;
use crate::entity::mob::piglin_ai::sample;

/// Vanilla `LongJumpMidJump.TIME_OUT_DURATION`.
const TIME_OUT_DURATION: i32 = 100;

/// Vanilla `LongJumpMidJump`: holds the long-jumping pose until the mob lands, then
/// starts the cooldown before the next jump.
pub struct LongJumpMidJump {
    time_between_long_jumps: (i32, i32),
    landing_sound: Sound,
    conditions: [(MemoryModuleId, MemoryStatus); 2],
}

impl LongJumpMidJump {
    #[must_use]
    pub const fn new(time_between_long_jumps: (i32, i32), landing_sound: Sound) -> Self {
        Self {
            time_between_long_jumps,
            landing_sound,
            conditions: [
                (types::LOOK_TARGET.id(), MemoryStatus::Registered),
                (types::LONG_JUMP_MID_JUMP.id(), MemoryStatus::ValuePresent),
            ],
        }
    }
}

impl Behavior for LongJumpMidJump {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn min_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn max_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        !tick.mob.get_entity().on_ground.load(Relaxed)
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        tick.mob
            .get_mob_entity()
            .living_entity
            .discard_friction
            .store(true, Relaxed);
        tick.mob.set_pose(EntityPose::LongJumping);
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        let living = &tick.mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        if entity.on_ground.load(Relaxed) {
            let velocity = entity.velocity.load();
            entity
                .velocity
                .store(velocity.multiply(f64::from(0.1f32), 1.0, f64::from(0.1f32)));
            entity.velocity_dirty.store(true, SeqCst);
            tick.world.play_sound_fine(
                self.landing_sound,
                SoundCategory::Neutral,
                &entity.pos.load(),
                2.0,
                1.0,
            );
        }

        living.discard_friction.store(false, Relaxed);
        tick.mob.set_pose(EntityPose::Standing);
        tick.brain.erase(types::LONG_JUMP_MID_JUMP.id());
        let cooldown = sample(self.time_between_long_jumps, &mut tick.mob.get_random());
        tick.brain.set(types::LONG_JUMP_COOLDOWN_TICKS, cooldown);
    }

    fn debug_name(&self) -> &'static str {
        "LongJumpMidJump"
    }
}
