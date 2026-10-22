//! Random target positions for goals.
//!
//! Locking: every helper here checks `goal_utils::has_malus` (and the land/default ones
//! `goal_utils::is_not_stable`), which lock the navigation `with_navigation` hands out (the
//! controlled vehicle's while riding one; for malus, the lending vehicle's). Never call them
//! inside a `with_navigation` closure: draw the position first, then enter the closure.

use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::{goal_utils, random_pos};
use crate::entity::ai::pathfinder::pathfinding_context::PathfindingContext;
use crate::entity::mob::Mob;
use crate::world::World;

/// Picks a spot in open air or water, at `flying_height` blocks off the mob's level.
#[must_use]
pub fn get_pos(
    mob: &dyn Mob,
    horizontal_dist: i32,
    vertical_dist: i32,
    flying_height: i32,
    x_dir: f64,
    z_dir: f64,
    max_xz_radians_difference: f64,
) -> Option<Vector3<f64>> {
    let restrict = goal_utils::mob_restricted(mob, f64::from(horizontal_dist));
    let world = mob.get_entity().world.load_full();
    let block_pos = mob.get_entity().block_pos.load();
    let mut context = PathfindingContext::new(
        Vector3::new(block_pos.0.x, block_pos.0.y, block_pos.0.z),
        world.clone(),
    );

    random_pos::generate_random_pos(
        || {
            generate_random_pos(
                mob,
                &world,
                &mut context,
                horizontal_dist,
                vertical_dist,
                flying_height,
                x_dir,
                z_dir,
                max_xz_radians_difference,
                restrict,
            )
        },
        |pos| f64::from(mob.get_walk_target_value(pos)),
    )
}

/// One candidate of vanilla `AirAndWaterRandomPos.generateRandomPos`.
#[expect(clippy::too_many_arguments, reason = "mirrors the upstream signature")]
pub fn generate_random_pos(
    mob: &dyn Mob,
    world: &World,
    context: &mut PathfindingContext,
    horizontal_dist: i32,
    vertical_dist: i32,
    flying_height: i32,
    x_dir: f64,
    z_dir: f64,
    max_xz_radians_difference: f64,
    restrict: bool,
) -> Option<BlockPos> {
    let max_y = world.dimension.min_y + world.dimension.height - 1;
    let direction = random_pos::generate_random_direction_within_radians(
        &mut *mob.get_entity().random(),
        0.0,
        f64::from(horizontal_dist),
        vertical_dist,
        flying_height,
        x_dir,
        z_dir,
        max_xz_radians_difference,
    )?;
    let pos = random_pos::generate_random_pos_toward_direction(
        mob,
        f64::from(horizontal_dist),
        &mut *mob.get_entity().random(),
        direction,
    );
    if goal_utils::is_outside_limits(&pos, world) || goal_utils::is_restricted(restrict, mob, &pos)
    {
        return None;
    }
    let pos = random_pos::move_up_out_of_solid(pos, max_y, |candidate| {
        goal_utils::is_solid(world, candidate)
    });
    (!goal_utils::has_malus(mob, context, &pos)).then_some(pos)
}
