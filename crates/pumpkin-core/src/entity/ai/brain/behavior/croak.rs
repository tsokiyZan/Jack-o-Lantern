use pumpkin_data::entity::EntityPose;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;

/// Vanilla `Croak.CROAK_TICKS`.
const CROAK_TICKS: i32 = 60;
/// Vanilla `Croak.TIME_OUT_DURATION`.
const TIME_OUT_DURATION: i32 = 100;

const ENTRY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 1] =
    [(types::WALK_TARGET.id(), MemoryStatus::ValueAbsent)];

/// Vanilla `Croak`: a frog standing out of liquid croaks for three seconds.
#[derive(Default)]
pub struct Croak {
    croak_counter: i32,
}

impl Behavior for Croak {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &ENTRY_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn max_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        tick.mob.get_entity().pose.load() == EntityPose::Standing
    }

    fn can_still_use(&mut self, _tick: &BrainTick<'_>) -> bool {
        self.croak_counter < CROAK_TICKS
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if !tick.mob.get_entity().is_in_liquid() {
            tick.mob.set_pose(EntityPose::Croaking);
            self.croak_counter = 0;
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        tick.mob.set_pose(EntityPose::Standing);
    }

    fn tick(&mut self, _tick: &mut BrainTick<'_>) {
        self.croak_counter += 1;
    }

    fn debug_name(&self) -> &'static str {
        "Croak"
    }
}
