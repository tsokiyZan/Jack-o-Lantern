//! Vanilla `ShootWhenStuck`.

use pumpkin_data::effect::StatusEffect;

use crate::entity::ai::brain::BrainTick;
use crate::entity::ai::brain::behavior::Behavior;
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};

/// How long vanilla `ShootWhenStuck.start` readies a shot for.
const SHOOT_WHEN_STUCK_TICKS: i64 = 60;

const ENTRY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 5] = [
    (types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent),
    (types::BREEZE_JUMP_INHALING.id(), MemoryStatus::ValueAbsent),
    (types::BREEZE_JUMP_TARGET.id(), MemoryStatus::ValueAbsent),
    (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
    (types::BREEZE_SHOOT.id(), MemoryStatus::ValueAbsent),
];

/// Vanilla `ShootWhenStuck`: a breeze that can neither slide nor jump (riding, in water,
/// levitating) readies a shot instead.
pub struct ShootWhenStuck;

impl Behavior for ShootWhenStuck {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &ENTRY_CONDITIONS
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        let entity = tick.mob.get_entity();
        entity.has_vehicle()
            || entity.is_in_water()
            || tick
                .mob
                .get_mob_entity()
                .living_entity
                .has_effect(&StatusEffect::LEVITATION)
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        tick.brain
            .set_with_expiry(types::BREEZE_SHOOT, (), SHOOT_WHEN_STUCK_TICKS);
    }

    fn debug_name(&self) -> &'static str {
        "ShootWhenStuck"
    }
}
