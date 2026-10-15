//! Vanilla `MoveBackToVillageGoal`: outside a village, stroll towards the nearest village
//! section.

use pumpkin_util::math::vector3::Vector3;

use super::wander_around::WanderAroundGoal;
use crate::entity::ai::util::default_random_pos;
use crate::entity::mob::Mob;
use crate::world::World;

/// Vanilla `MoveBackToVillageGoal.MAX_XZ_DIST`.
const MAX_XZ_DIST: i32 = 10;
/// Vanilla `MoveBackToVillageGoal.MAX_Y_DIST`.
const MAX_Y_DIST: i32 = 7;

/// Vanilla `new MoveBackToVillageGoal(mob, speedModifier, checkNoActionTime)`: a random
/// stroll with interval 10.
#[must_use]
pub fn new(speed: f64, check_no_action_time: bool) -> WanderAroundGoal {
    let goal = WanderAroundGoal::with_interval(speed, 10)
        .with_position(position)
        .starting_only_when(not_in_village);
    if check_no_action_time {
        goal
    } else {
        goal.ignoring_no_action_time()
    }
}

/// `canUse`: only outside a village.
fn not_in_village(mob: &dyn Mob) -> bool {
    let entity = mob.get_entity();
    !entity.world.load().is_village(&entity.block_pos.load())
}

/// `getPosition`: towards the centre of the section nearest a village, if one is closer.
fn position(mob: &dyn Mob, _probability: f32) -> Option<Vector3<f64>> {
    let entity = mob.get_entity();
    let pos = entity.block_pos.load();
    let section = Vector3::new(pos.0.x >> 4, pos.0.y >> 4, pos.0.z >> 4);
    let optimal = find_section_closest_to_village(&entity.world.load(), section, 2);
    if optimal == section {
        return None;
    }
    // `Vec3.atBottomCenterOf(optimalSectionPos.center())`.
    let center = Vector3::new(
        f64::from((optimal.x << 4) + 8) + 0.5,
        f64::from((optimal.y << 4) + 8),
        f64::from((optimal.z << 4) + 8) + 0.5,
    );
    default_random_pos::get_pos_towards(
        mob,
        MAX_XZ_DIST,
        MAX_Y_DIST,
        center,
        std::f64::consts::FRAC_PI_2,
    )
}

/// Vanilla `BehaviorUtils.findSectionClosestToVillage`: the first section of the cube, in
/// `SectionPos.cube` order (x fastest, then y, then z), with the fewest sections to a village,
/// if fewer than `center`'s.
fn find_section_closest_to_village(
    world: &World,
    center: Vector3<i32>,
    radius: i32,
) -> Vector3<i32> {
    let pois = world
        .villager_poi
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dist_to_village = pois.sections_to_village(center);
    let mut best: Option<(i32, Vector3<i32>)> = None;
    for z in center.z - radius..=center.z + radius {
        for y in center.y - radius..=center.y + radius {
            for x in center.x - radius..=center.x + radius {
                let section = Vector3::new(x, y, z);
                let dist = pois.sections_to_village(section);
                if dist < dist_to_village && best.is_none_or(|(best_dist, _)| dist < best_dist) {
                    best = Some((dist, section));
                }
            }
        }
    }
    best.map_or(center, |(_, section)| section)
}
