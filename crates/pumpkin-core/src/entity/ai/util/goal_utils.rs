use std::sync::Arc;

use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::EntityBase;
use crate::entity::ai::pathfinder::pathfinding_context::PathfindingContext;
use crate::entity::mob::Mob;
use crate::world::World;

/// Vanilla `getEntitiesOfClass(LivingEntity.class, area, e -> true)`.
#[must_use]
pub fn get_living_entities_in_area(world: &World, area: &BoundingBox) -> Vec<Arc<dyn EntityBase>> {
    let mut found = world
        .get_entities_at_box(area)
        .into_iter()
        .filter(|entity| entity.get_living_entity().is_some())
        .collect::<Vec<_>>();
    found.extend(
        world
            .get_players_at_box(area)
            .into_iter()
            .map(|player| player as Arc<dyn EntityBase>),
    );
    found.sort_by_cached_key(|e| crate::entity::section_iteration_key(e.get_entity()));
    found
}

/// The vanilla entity class `entity_type`'s class extends, when that parent is itself a
/// concrete entity type.
const fn parent_class(entity_type: &EntityType) -> Option<&'static EntityType> {
    let id = entity_type.id;
    if id == EntityType::CAVE_SPIDER.id {
        Some(&EntityType::SPIDER)
    } else if id == EntityType::DROWNED.id
        || id == EntityType::HUSK.id
        || id == EntityType::ZOMBIE_VILLAGER.id
        || id == EntityType::ZOMBIFIED_PIGLIN.id
    {
        Some(&EntityType::ZOMBIE)
    } else if id == EntityType::ELDER_GUARDIAN.id {
        Some(&EntityType::GUARDIAN)
    } else if id == EntityType::GLOW_SQUID.id {
        Some(&EntityType::SQUID)
    } else if id == EntityType::TRADER_LLAMA.id {
        Some(&EntityType::LLAMA)
    } else if id == EntityType::CAMEL_HUSK.id {
        Some(&EntityType::CAMEL)
    } else {
        None
    }
}

/// Vanilla `Class.isInstance` for a class named by its own entity type: entities of a
/// subclass match too, e.g. a husk is a `Zombie` and a trader llama is a `Llama`.
#[must_use]
pub fn is_instance_of(class: &EntityType, entity_type: &EntityType) -> bool {
    entity_type == class || parent_class(entity_type).is_some_and(|parent| parent == class)
}

/// Vanilla `AbstractHorse` class membership.
#[must_use]
pub fn is_abstract_horse(entity_type: &'static EntityType) -> bool {
    [
        &EntityType::HORSE,
        &EntityType::DONKEY,
        &EntityType::MULE,
        &EntityType::SKELETON_HORSE,
        &EntityType::ZOMBIE_HORSE,
        &EntityType::CAMEL,
        &EntityType::CAMEL_HUSK,
        &EntityType::LLAMA,
        &EntityType::TRADER_LLAMA,
    ]
    .contains(&entity_type)
}

/// Vanilla `getEntitiesOfClass(class, area, e -> true)`. `None` stands for `Mob`.
#[must_use]
pub fn get_entities_of_class(
    world: &World,
    class: Option<&EntityType>,
    area: &BoundingBox,
) -> Vec<Arc<dyn EntityBase>> {
    if class == Some(&EntityType::PLAYER) {
        return world
            .get_players_at_box(area)
            .into_iter()
            .map(|player| player as Arc<dyn EntityBase>)
            .collect();
    }
    world
        .get_entities_at_box(area)
        .into_iter()
        .filter(|entity| {
            class.map_or_else(
                || entity.get_mob().is_some(),
                |class| is_instance_of(class, entity.get_entity().entity_type),
            )
        })
        .collect()
}

/// Vanilla `GoalUtils.hasGroundPathNavigation`: `getNavigation()`, so a steered vehicle's.
/// Takes that navigation's lock: never call it inside a `with_navigation` closure.
#[must_use]
pub fn has_ground_path_navigation(mob: &dyn Mob) -> bool {
    mob.get_mob_entity()
        .with_navigation(|navigation, _| navigation.can_navigate_ground())
}

#[must_use]
pub fn mob_restricted(mob: &dyn Mob, horizontal_dist: f64) -> bool {
    let mob_entity = mob.get_mob_entity();
    if !mob_entity.has_position_target() {
        return false;
    }
    let home = mob_entity.position_target.load();
    let radius = f64::from(
        mob_entity
            .position_target_range
            .load(std::sync::atomic::Ordering::Relaxed),
    );
    let limit = radius + horizontal_dist + 1.0;
    home.to_centered_f64()
        .squared_distance_to_vec(&mob_entity.living_entity.entity.pos.load())
        < limit * limit
}

#[must_use]
pub const fn is_outside_limits(pos: &BlockPos, world: &World) -> bool {
    pos.0.y < world.dimension.min_y || pos.0.y >= world.dimension.min_y + world.dimension.height
}

#[must_use]
pub fn is_restricted(restrict: bool, mob: &dyn Mob, pos: &BlockPos) -> bool {
    restrict && !mob.get_mob_entity().is_in_position_target_range_pos(pos)
}

/// Nowhere the mob's navigation can settle, which for a walker means nothing
/// solid to stand on and for a swimmer means inside a block.
///
/// Asks `getNavigation()` through `with_navigation`, so it locks the controlled vehicle's
/// navigation while the mob steers one, else its own. Callers must not already hold that
/// lock: never call this inside a `with_navigation` closure.
#[must_use]
pub fn is_not_stable(mob: &dyn Mob, world: &World, pos: &BlockPos) -> bool {
    !mob.get_mob_entity()
        .with_navigation(|navigation, _| navigation.is_stable_destination(world, pos))
}

#[must_use]
pub fn is_water(world: &World, pos: &BlockPos) -> bool {
    let (_, state_id) = world.get_block_and_state_id(pos);
    if state_id.to_state().is_waterlogged() {
        return true;
    }
    pumpkin_data::fluid::Fluid::from_state_id(state_id)
        .is_some_and(|fluid| fluid.has_tag(&tag::Fluid::MINECRAFT_WATER))
}

/// Vanilla `GoalUtils.hasMalus`, typed by `WalkNodeEvaluator.getPathTypeStatic`.
///
/// Takes the navigation lock (the vehicle's, while the mob inherits its malus), so callers
/// must not already hold it: never call this inside a `with_navigation` closure. Check malus
/// before entering the closure.
#[must_use]
pub fn has_malus(mob: &dyn Mob, context: &mut PathfindingContext, pos: &BlockPos) -> bool {
    let path_type = context.get_land_node_type(Vector3::new(pos.0.x, pos.0.y, pos.0.z));
    let malus = mob.get_mob_entity().get_pathfinding_malus(path_type);
    malus != 0.0
}

#[must_use]
pub fn is_solid(world: &World, pos: &BlockPos) -> bool {
    world.get_block_state(pos).is_solid()
}

const ALERT_RANGE_Y: f64 = 10.0;

/// Nearby mobs of `mob`'s class that are free to take over its grievance.
///
/// Vanilla `getEntitiesOfClass(mob.getClass(), ...)`, so subclasses count too. Anchored on a
/// unit cube at the mob's feet, so a wide hitbox does not widen the call.
pub fn nearby_same_type(mob: &dyn Mob) -> Vec<Arc<dyn EntityBase>> {
    use crate::entity::predicate::EntityPredicate;
    use pumpkin_data::attributes::Attributes;
    use pumpkin_util::math::boundingbox::BoundingBox;

    let mob_entity = mob.get_mob_entity();
    let entity = &mob_entity.living_entity.entity;
    let within = mob_entity
        .living_entity
        .get_attribute_value(&Attributes::FOLLOW_RANGE);

    let world = entity.world.load();
    let pos = entity.pos.load();
    let search_box =
        BoundingBox::new(pos, pos.add_raw(1.0, 1.0, 1.0)).expand(within, ALERT_RANGE_Y, within);

    world
        .get_entities_at_box(&search_box)
        .into_iter()
        .filter(|other| {
            let other_entity = other.get_entity();
            other_entity.entity_id != entity.entity_id
                && is_instance_of(entity.entity_type, other_entity.entity_type)
                && EntityPredicate::ExceptSpectator.test(other_entity)
        })
        .collect()
}

/// Vanilla `BlockPos.withinBoxByManhattanDistance`: every position in the box, in
/// order of Manhattan distance, in vanilla's exact iteration order.
#[must_use]
pub fn within_box_by_manhattan_distance(
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

#[cfg(test)]
mod tests {
    use super::within_box_by_manhattan_distance;
    use pumpkin_util::math::position::BlockPos;

    #[test]
    fn manhattan_box_visits_positive_z_before_negative() {
        // First seven positions of vanilla `withinBoxByManhattanDistance(origin, 5, 1, 5)`.
        let expected = [
            (0, 0, 0),
            (-1, 0, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
            (0, 1, 0),
            (1, 0, 0),
        ];
        let visited = within_box_by_manhattan_distance(BlockPos::new(0, 0, 0), 5, 1, 5);
        for (pos, (x, y, z)) in visited.iter().zip(expected) {
            assert_eq!((pos.0.x, pos.0.y, pos.0.z), (x, y, z));
        }
    }
}
