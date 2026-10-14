use std::sync::Arc;

use super::super::BrainTick;
use super::super::memory::position_tracker::{EntityTracker, PositionTracker};
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;
use crate::entity::EntityBase;

/// Vanilla `FollowTemptation.TEMPTATION_COOLDOWN`.
pub const TEMPTATION_COOLDOWN: i32 = 100;
/// Vanilla `FollowTemptation.DEFAULT_CLOSE_ENOUGH_DIST`.
pub const DEFAULT_CLOSE_ENOUGH_DIST: f64 = 2.5;

type SpeedModifier = Box<dyn Fn(&BrainTick<'_>) -> f32 + Send + Sync>;

/// Vanilla `FollowTemptation`: walks after the player holding something the mob
/// wants, and stops for a cooldown once that player goes.
pub struct FollowTemptation {
    speed_modifier: SpeedModifier,
    close_enough_distance: f64,
    look_in_the_eyes: bool,
    conditions: [(MemoryModuleId, MemoryStatus); 6],
}

impl FollowTemptation {
    #[must_use]
    pub fn new(speed_modifier: impl Fn(&BrainTick<'_>) -> f32 + Send + Sync + 'static) -> Self {
        Self {
            speed_modifier: Box::new(speed_modifier),
            close_enough_distance: DEFAULT_CLOSE_ENOUGH_DIST,
            look_in_the_eyes: false,
            conditions: [
                (types::LOOK_TARGET.id(), MemoryStatus::Registered),
                (types::WALK_TARGET.id(), MemoryStatus::Registered),
                (
                    types::TEMPTATION_COOLDOWN_TICKS.id(),
                    MemoryStatus::ValueAbsent,
                ),
                (types::TEMPTING_PLAYER.id(), MemoryStatus::ValuePresent),
                (types::BREED_TARGET.id(), MemoryStatus::ValueAbsent),
                (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
            ],
        }
    }
}

impl Behavior for FollowTemptation {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn times_out(&self) -> bool {
        false
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        tick.brain.has_memory_value(types::TEMPTING_PLAYER.id())
            && !tick.brain.has_memory_value(types::BREED_TARGET.id())
            && !tick.brain.has_memory_value(types::IS_PANICKING.id())
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        tick.brain
            .set(types::TEMPTATION_COOLDOWN_TICKS, TEMPTATION_COOLDOWN);
        tick.brain.erase(types::WALK_TARGET.id());
        tick.brain.erase(types::LOOK_TARGET.id());
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(player) = tick.brain.get(types::TEMPTING_PLAYER).map(Arc::clone) else {
            return;
        };
        let player: Arc<dyn EntityBase> = player;
        tick.brain.set(
            types::LOOK_TARGET,
            Arc::new(EntityTracker::new(Arc::clone(&player), true)) as Arc<dyn PositionTracker>,
        );
        let distance_sq = tick
            .mob
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&player.get_entity().pos.load());
        if distance_sq < self.close_enough_distance * self.close_enough_distance {
            tick.brain.erase(types::WALK_TARGET.id());
        } else {
            let speed_modifier = (self.speed_modifier)(tick);
            tick.brain.set(
                types::WALK_TARGET,
                WalkTarget::new(
                    Arc::new(EntityTracker::new(player, self.look_in_the_eyes)),
                    speed_modifier,
                    2,
                ),
            );
        }
    }

    fn debug_name(&self) -> &'static str {
        "FollowTemptation"
    }
}
