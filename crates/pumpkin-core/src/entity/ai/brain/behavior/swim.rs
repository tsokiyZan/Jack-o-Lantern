use rand::RngExt;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, MemoryStatus};
use super::timed::Behavior;
use crate::entity::ai::goal::swim::SwimGoal;

/// Vanilla `Swim`: the brain's `FloatGoal`, jumping with `chance` each tick while in
/// deep enough fluid.
pub struct Swim {
    chance: f32,
}

impl Swim {
    #[must_use]
    pub const fn new(chance: f32) -> Self {
        Self { chance }
    }
}

impl Behavior for Swim {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &[]
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        SwimGoal::is_in_fluid(tick.mob)
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        SwimGoal::is_in_fluid(tick.mob)
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        if tick.mob.get_random().random::<f32>() < self.chance {
            tick.mob.get_mob_entity().jump();
        }
    }

    fn debug_name(&self) -> &'static str {
        "Swim"
    }
}
