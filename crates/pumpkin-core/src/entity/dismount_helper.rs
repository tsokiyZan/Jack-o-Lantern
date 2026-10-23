//! Vanilla `DismountHelper`: where an entity may stand after leaving a vehicle or a bed.

use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockState};
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use crate::world::World;
use crate::world::natural_spawner::is_block_dangerous;

/// The top of a block's shapes, vanilla `VoxelShape.max(Direction.Axis.Y)`: negative
/// infinity for an empty shape.
fn max_y(shapes: impl Iterator<Item = BoundingBox>) -> f64 {
    shapes
        .map(|shape| shape.max.y)
        .fold(f64::NEG_INFINITY, f64::max)
}

/// Vanilla `DismountHelper.nonClimbableShape`: the collision shape's top, or nothing for a
/// `#climbable` block or an open trapdoor.
fn non_climbable_shape_top(world: &World, pos: &BlockPos) -> f64 {
    let state = world.get_block_state(pos);
    let block = Block::from_state_id(state.id);
    let open_trapdoor = block.has_tag(&tag::Block::MINECRAFT_TRAPDOORS)
        && pumpkin_data::block_properties::OakTrapdoorLikeProperties::from_state_id(state.id).open;
    if block.has_tag(&tag::Block::MINECRAFT_CLIMBABLE) || open_trapdoor {
        return f64::NEG_INFINITY;
    }
    max_y(state.get_block_collision_shapes_at(pos))
}

/// Vanilla `BlockGetter.getBlockFloorHeight(shape, below)`, with each shape given by its top.
fn block_floor_height(block_top: f64, below_top: impl FnOnce() -> f64) -> f64 {
    if block_top != f64::NEG_INFINITY {
        return block_top;
    }
    let below_floor = below_top();
    if below_floor >= 1.0 {
        below_floor - 1.0
    } else {
        f64::NEG_INFINITY
    }
}

/// Vanilla `DismountHelper.isBlockFloorValid`.
#[must_use]
pub fn is_block_floor_valid(block_floor_height: f64) -> bool {
    !block_floor_height.is_infinite() && block_floor_height < 1.0
}

/// Vanilla `DismountHelper.findSafeDismountLocation`.
#[must_use]
pub fn find_safe_dismount_location(
    entity_type: &'static EntityType,
    world: &World,
    pos: BlockPos,
    check_dangerous: bool,
) -> Option<Vector3<f64>> {
    let state: &'static BlockState = world.get_block_state(&pos);
    if check_dangerous && is_block_dangerous(entity_type, state) {
        return None;
    }
    let below = pos.down();
    let floor_height = block_floor_height(non_climbable_shape_top(world, &pos), || {
        non_climbable_shape_top(world, &below)
    });
    if !is_block_floor_valid(floor_height) {
        return None;
    }
    if check_dangerous
        && floor_height <= 0.0
        && is_block_dangerous(entity_type, world.get_block_state(&below))
    {
        return None;
    }

    // `Vec3.upFromBottomCenterOf(blockPos, floorHeight)`.
    let position = Vector3::new(
        f64::from(pos.0.x) + 0.5,
        f64::from(pos.0.y) + floor_height,
        f64::from(pos.0.z) + 0.5,
    );
    let dimensions = EntityDimensions::new(
        entity_type.dimension[0],
        entity_type.dimension[1],
        entity_type.eye_height,
    );
    let aabb = BoundingBox::new_from_pos(position.x, position.y, position.z, &dimensions);
    // `level.getBlockCollisions(null, aabb)`: blocks only, no entity context.
    if !world.is_space_empty(aabb) {
        return None;
    }

    if entity_type == &EntityType::PLAYER
        && (Block::from_state_id(state.id).has_tag(&tag::Block::MINECRAFT_INVALID_SPAWN_INSIDE)
            || world
                .get_block(&pos.up())
                .has_tag(&tag::Block::MINECRAFT_INVALID_SPAWN_INSIDE))
    {
        return None;
    }

    let inside_border = world
        .worldborder
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_box(&aabb);
    inside_border.then_some(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_height_falls_back_to_the_block_below() {
        // A full block below an empty one: the floor is the top of the block below, 0.
        assert_eq!(block_floor_height(f64::NEG_INFINITY, || 1.0), 0.0);
        // A fence below (1.5 high) reaches half a block up.
        assert_eq!(block_floor_height(f64::NEG_INFINITY, || 1.5), 0.5);
        // A slab below doesn't reach: no floor.
        assert_eq!(
            block_floor_height(f64::NEG_INFINITY, || 0.5),
            f64::NEG_INFINITY
        );
        // A carpet at the position itself is the floor.
        assert_eq!(block_floor_height(0.0625, || panic!("not read")), 0.0625);
        assert!(!is_block_floor_valid(f64::NEG_INFINITY));
        assert!(!is_block_floor_valid(1.0));
        assert!(is_block_floor_valid(0.5));
    }
}
