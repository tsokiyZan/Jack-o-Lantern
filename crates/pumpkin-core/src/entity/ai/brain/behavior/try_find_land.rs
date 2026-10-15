use crate::entity::ai::util::goal_utils;
use std::sync::Arc;

use pumpkin_data::BlockDirection;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;

use super::super::memory::position_tracker::{BlockPosTracker, PositionTracker};
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryStatus, types};
use super::one_shot::OneShot;
use crate::world::World;

/// Vanilla `TryFindLand.COOLDOWN_TICKS`.
const COOLDOWN_TICKS: i64 = 60;

/// Vanilla `TryFindLand`: in water, walks to the nearest dry spot within `range`
/// (Manhattan) that it can stand on.
#[must_use]
pub fn try_find_land(range: i32, speed_modifier: f32) -> OneShot {
    let mut next_ok_start_time: i64 = 0;
    OneShot::with_required(
        "TryFindLand",
        vec![
            (types::ATTACK_TARGET.id(), MemoryStatus::ValueAbsent),
            (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
        ],
        vec![types::LOOK_TARGET.id()],
        move |tick| {
            let body_pos = tick.mob.get_entity().block_pos.load();
            let world = Arc::clone(tick.world);
            if !world
                .get_fluid_type(&body_pos)
                .has_tag(&tag::Fluid::MINECRAFT_WATER)
            {
                return false;
            }
            if tick.time < next_ok_start_time {
                next_ok_start_time = tick.time + COOLDOWN_TICKS;
                return true;
            }
            let found = goal_utils::within_box_by_manhattan_distance(body_pos, range, range, range)
                .into_iter()
                .filter(|pos| pos.0.x != body_pos.0.x || pos.0.z != body_pos.0.z)
                .filter(|pos| world.get_fluid_type(pos).id == pumpkin_data::fluid::Fluid::EMPTY.id)
                .find(|pos| can_stand_on(&world, pos));
            if let Some(pos) = found {
                tick.brain.set(
                    types::LOOK_TARGET,
                    Arc::new(BlockPosTracker::new(pos)) as Arc<dyn PositionTracker>,
                );
                tick.brain.set(
                    types::WALK_TARGET,
                    WalkTarget::new(Arc::new(BlockPosTracker::new(pos)), speed_modifier, 1),
                );
            }
            next_ok_start_time = tick.time + COOLDOWN_TICKS;
            true
        },
    )
}

/// Vanilla `TryFindLand.canStandOn`.
fn can_stand_on(world: &World, pos: &BlockPos) -> bool {
    has_no_collision(world, pos)
        && world
            .get_block_state(&pos.down())
            .is_side_solid(BlockDirection::Up)
}

/// Whether `pos` has an empty collision shape. Vanilla asks with the mob's
/// `CollisionContext`, which only matters for scaffolding and powder snow.
pub(super) fn has_no_collision(world: &World, pos: &BlockPos) -> bool {
    world
        .get_block_state(pos)
        .get_block_collision_shapes()
        .next()
        .is_none()
}
