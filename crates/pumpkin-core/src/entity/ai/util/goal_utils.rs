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
/// Takes the navigation lock, so callers must not already hold it.
#[must_use]
pub fn is_not_stable(mob: &dyn Mob, world: &World, pos: &BlockPos) -> bool {
    !mob.get_mob_entity()
        .navigator
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_stable_destination(world, pos)
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

/// Takes the navigation lock, so callers must not already hold it.
#[must_use]
pub fn has_malus(mob: &dyn Mob, context: &mut PathfindingContext, pos: &BlockPos) -> bool {
    let path_type = context.get_path_type_from_state(Vector3::new(pos.0.x, pos.0.y, pos.0.z));
    let malus = mob
        .get_mob_entity()
        .navigator
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_pathfinding_malus(path_type);
    malus != 0.0
}

#[must_use]
pub fn is_solid(world: &World, pos: &BlockPos) -> bool {
    world.get_block_state(pos).is_solid()
}

const ALERT_RANGE_Y: f64 = 10.0;

/// Nearby mobs of the same type that are free to take over `mob`'s grievance.
/// Anchored on a unit cube at the mob's feet, so a wide hitbox does not widen the call.
pub fn nearby_same_type(mob: &dyn Mob) -> Vec<std::sync::Arc<dyn crate::entity::EntityBase>> {
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
                && other_entity.entity_type == entity.entity_type
                && EntityPredicate::ExceptSpectator.test(other_entity)
        })
        .collect()
}
