use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryStatus, types};
use super::one_shot::OneShot;
use crate::entity::ai::util::{default_random_pos, goal_utils, land_random_pos};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

const MAX_XZ_DIST: i32 = 10;
const MAX_Y_DIST: i32 = 7;

#[must_use]
pub fn stroll(speed_modifier: f32, may_stroll_from_water: bool) -> OneShot {
    stroll_with_range(
        speed_modifier,
        MAX_XZ_DIST,
        MAX_Y_DIST,
        may_stroll_from_water,
    )
}

#[must_use]
pub fn stroll_with_range(
    speed_modifier: f32,
    max_horizontal_distance: i32,
    max_vertical_distance: i32,
    may_stroll_from_water: bool,
) -> OneShot {
    OneShot::new(
        "RandomStroll",
        vec![(types::WALK_TARGET.id(), MemoryStatus::ValueAbsent)],
        move |tick| {
            if !may_stroll_from_water && tick.mob.get_entity().is_in_water() {
                return false;
            }
            let target =
                land_random_pos::get_pos(tick.mob, max_horizontal_distance, max_vertical_distance)
                    .map(|pos| WalkTarget::from_vec(pos, speed_modifier, 0));
            tick.brain.set_optional(types::WALK_TARGET, target);
            true
        },
    )
}

/// Vanilla `RandomStroll.SWIM_XY_DISTANCE_TIERS`.
const SWIM_XY_DISTANCE_TIERS: [(i32, i32); 6] = [(1, 1), (3, 3), (5, 5), (6, 5), (7, 7), (10, 7)];

/// Vanilla `RandomStroll.swim`: only while in water, to a swimmable spot.
#[must_use]
pub fn swim(speed_modifier: f32) -> OneShot {
    OneShot::new(
        "RandomStroll",
        vec![(types::WALK_TARGET.id(), MemoryStatus::ValueAbsent)],
        move |tick| {
            if !tick.mob.get_entity().is_in_water() {
                return false;
            }
            let target =
                target_swim_pos(tick.mob).map(|pos| WalkTarget::from_vec(pos, speed_modifier, 0));
            tick.brain.set_optional(types::WALK_TARGET, target);
            true
        },
    )
}

/// Vanilla `RandomStroll.getTargetSwimPos`: tries ever wider tiers, keeping the last
/// one that landed in a fluid and continuing in its direction.
fn target_swim_pos(mob: &dyn crate::entity::mob::Mob) -> Option<Vector3<f64>> {
    let world = mob.get_entity().world.load_full();
    let body = mob.get_entity().pos.load();
    let mut fallback: Option<Vector3<f64>> = None;
    let mut target: Option<Vector3<f64>> = None;
    for (horizontal, vertical) in SWIM_XY_DISTANCE_TIERS {
        target = fallback.map_or_else(
            || default_random_pos::get_swimmable_pos(mob, horizontal, vertical),
            |fallback| {
                let direction = (fallback - body).normalize();
                Some(Vector3::new(
                    body.x + direction.x * f64::from(horizontal),
                    body.y + direction.y * f64::from(vertical),
                    body.z + direction.z * f64::from(horizontal),
                ))
            },
        );
        let restrict = goal_utils::mob_restricted(mob, f64::from(horizontal));
        let Some(pos) = target else {
            return fallback;
        };
        let block_pos = BlockPos::floored(pos.x, pos.y, pos.z);
        if world.get_fluid(&block_pos).id == pumpkin_data::fluid::Fluid::EMPTY.id
            || goal_utils::is_restricted(restrict, mob, &block_pos)
        {
            return fallback;
        }
        fallback = Some(pos);
    }
    target
}
