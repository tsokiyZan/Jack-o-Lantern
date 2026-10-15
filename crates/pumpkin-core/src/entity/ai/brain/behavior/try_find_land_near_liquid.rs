use crate::entity::ai::util::goal_utils;
use std::sync::Arc;

use pumpkin_data::BlockDirection;
use pumpkin_data::tag::{Tag, Taggable};

use super::super::memory::position_tracker::{BlockPosTracker, PositionTracker};
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryStatus, types};
use super::one_shot::OneShot;
use super::try_find_land::has_no_collision;

/// Vanilla `TryFindLandNearLiquid`'s cooldown between searches.
const COOLDOWN_TICKS: i64 = 40;

/// Vanilla `TryFindLandNearLiquid`: out of `fluid_tag`, walks to the nearest open spot
/// within `range` (Manhattan) with ground under it and that fluid below an air block
/// beside it.
#[must_use]
pub fn try_find_land_near_liquid(
    range: i32,
    speed_modifier: f32,
    fluid_tag: &'static Tag,
) -> OneShot {
    let mut next_ok_start_time: i64 = 0;
    OneShot::with_required(
        "TryFindLandNearLiquid",
        vec![
            (types::ATTACK_TARGET.id(), MemoryStatus::ValueAbsent),
            (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
        ],
        vec![types::LOOK_TARGET.id()],
        move |tick| {
            let body_pos = tick.mob.get_entity().block_pos.load();
            let world = Arc::clone(tick.world);
            if world.get_fluid_type(&body_pos).has_tag(fluid_tag) {
                return false;
            }
            if tick.time < next_ok_start_time {
                next_ok_start_time = tick.time + COOLDOWN_TICKS;
                return true;
            }
            let found = goal_utils::within_box_by_manhattan_distance(body_pos, range, range, range)
                .into_iter()
                .filter(|pos| pos.0.x != body_pos.0.x || pos.0.z != body_pos.0.z)
                .find(|pos| {
                    if !has_no_collision(&world, pos) || has_no_collision(&world, &pos.down()) {
                        return false;
                    }
                    BlockDirection::horizontal().into_iter().any(|direction| {
                        let side = pos.offset(direction.to_offset());
                        world.get_block_state(&side).is_air()
                            && world.get_fluid_type(&side.down()).has_tag(fluid_tag)
                    })
                });
            if let Some(pos) = found {
                tick.brain.set(
                    types::LOOK_TARGET,
                    Arc::new(BlockPosTracker::new(pos)) as Arc<dyn PositionTracker>,
                );
                tick.brain.set(
                    types::WALK_TARGET,
                    WalkTarget::new(Arc::new(BlockPosTracker::new(pos)), speed_modifier, 0),
                );
            }
            next_ok_start_time = tick.time + COOLDOWN_TICKS;
            true
        },
    )
}
