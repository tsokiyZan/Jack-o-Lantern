use pumpkin_util::math::vector3::Vector3;

use super::{air_and_water_random_pos, goal_utils, random_pos};
use crate::entity::ai::pathfinder::pathfinding_context::PathfindingContext;
use crate::entity::mob::Mob;

/// Vanilla `AirRandomPos.getPosTowards`: a spot in open air, but not water, picked in
/// the direction of `towards_pos`.
#[must_use]
pub fn get_pos_towards(
    mob: &dyn Mob,
    horizontal_dist: i32,
    vertical_dist: i32,
    flying_height: i32,
    towards_pos: Vector3<f64>,
    max_xz_radians_from_dir: f64,
) -> Option<Vector3<f64>> {
    let mob_pos = mob.get_entity().pos.load();
    let dir = towards_pos.sub(&mob_pos);
    let restrict = goal_utils::mob_restricted(mob, f64::from(horizontal_dist));
    let world = mob.get_entity().world.load_full();
    let block_pos = mob.get_entity().block_pos.load();
    let mut context = PathfindingContext::new(
        Vector3::new(block_pos.0.x, block_pos.0.y, block_pos.0.z),
        world.clone(),
    );
    let mut rng = mob.get_random();

    random_pos::generate_random_pos(
        || {
            air_and_water_random_pos::generate_random_pos(
                mob,
                &world,
                &mut context,
                &mut rng,
                horizontal_dist,
                vertical_dist,
                flying_height,
                dir.x,
                dir.z,
                max_xz_radians_from_dir,
                restrict,
            )
            .filter(|pos| !goal_utils::is_water(&world, pos))
        },
        |pos| f64::from(mob.get_walk_target_value(pos)),
    )
}
