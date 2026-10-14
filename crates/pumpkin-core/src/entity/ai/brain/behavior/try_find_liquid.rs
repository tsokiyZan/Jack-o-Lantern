use std::sync::Arc;

use pumpkin_data::Block;
use pumpkin_data::tag::{Tag, Taggable};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::super::memory::position_tracker::{BlockPosTracker, PositionTracker};
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryStatus, types};
use super::one_shot::OneShot;

/// Vanilla `TryFindLiquid`: on dry land, walks to the nearest liquid of `fluid_tag`
/// within `range` (Manhattan), preferring one with air above it.
#[must_use]
pub fn try_find_liquid(range: i32, speed_modifier: f32, fluid_tag: &'static Tag) -> OneShot {
    let mut next_ok_start_time: i64 = 0;
    OneShot::with_required(
        "TryFindLiquid",
        vec![
            (types::ATTACK_TARGET.id(), MemoryStatus::ValueAbsent),
            (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
        ],
        vec![types::LOOK_TARGET.id()],
        move |tick| {
            let entity = tick.mob.get_entity();
            let body_pos = entity.block_pos.load();
            let world = Arc::clone(tick.world);
            if world.get_fluid_type(&body_pos).has_tag(fluid_tag) {
                return false;
            }
            if tick.time < next_ok_start_time {
                next_ok_start_time = tick.time + 22;
                return true;
            }

            let body = entity.pos.load();
            let mut found: Option<BlockPos> = None;
            for pos in within_box_by_manhattan_distance(body_pos, range, range, range) {
                if pos.0.x == body_pos.0.x && pos.0.z == body_pos.0.z {
                    continue;
                }
                let block = world.get_block(&pos);
                let is_liquid_block = block == &Block::WATER || block == &Block::LAVA;
                if !is_liquid_block || !world.get_fluid_type(&pos).has_tag(fluid_tag) {
                    continue;
                }
                if world.get_block_state(&pos.up()).is_air() {
                    found = Some(pos);
                    break;
                }
                if found.is_none() && !closer_to_center_than(&pos, body, 1.5) {
                    found = Some(pos);
                }
            }
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
            next_ok_start_time = tick.time + 40;
            true
        },
    )
}

/// Vanilla `Vec3i.closerToCenterThan`.
fn closer_to_center_than(pos: &BlockPos, point: Vector3<f64>, distance: f64) -> bool {
    let center = Vector3::new(
        f64::from(pos.0.x) + 0.5,
        f64::from(pos.0.y) + 0.5,
        f64::from(pos.0.z) + 0.5,
    );
    center.squared_distance_to_vec(&point) < distance * distance
}

/// Vanilla `BlockPos.withinBoxByManhattanDistance`: every position in the box, in
/// order of Manhattan distance, in vanilla's exact iteration order.
fn within_box_by_manhattan_distance(
    origin: BlockPos,
    reach_x: i32,
    reach_y: i32,
    reach_z: i32,
) -> Vec<BlockPos> {
    let max_depth = reach_x + reach_y + reach_z;
    let mut out = Vec::new();
    let (mut depth, mut max_x, mut max_y, mut x, mut y) = (0, 0, 0, 0, 0);
    loop {
        let found = loop {
            if y > max_y {
                x += 1;
                if x > max_x {
                    depth += 1;
                    if depth > max_depth {
                        return out;
                    }
                    max_x = reach_x.min(depth);
                    x = -max_x;
                }
                max_y = reach_y.min(depth - x.abs());
                y = -max_y;
            }
            let z = depth - x.abs() - y.abs();
            let candidate = (z <= reach_z).then_some((x, y, z));
            y += 1;
            if let Some(found) = candidate {
                break found;
            }
        };
        let (dx, dy, dz) = found;
        out.push(BlockPos::new(
            origin.0.x + dx,
            origin.0.y + dy,
            origin.0.z + dz,
        ));
        if dz != 0 {
            out.push(BlockPos::new(
                origin.0.x + dx,
                origin.0.y + dy,
                origin.0.z - dz,
            ));
        }
    }
}
