use std::sync::Arc;

use pumpkin_data::{
    Block, BlockState,
    entity::EntityType,
    tag::{self, Taggable},
};
use pumpkin_util::math::wrap_degrees;
use pumpkin_util::math::{
    boundingbox::{BoundingBox, EntityDimensions},
    position::BlockPos,
    vector3::Vector3,
};
use pumpkin_util::random::RandomImpl;
use uuid::Uuid;

use crate::entity::{Entity, EntityBase, mob::Mob};
use crate::world::World;

#[derive(Clone, Copy)]
pub enum SpawnStrategy {
    OnTopOfCollider,
    OnTopOfColliderNoLeaves,
}

impl SpawnStrategy {
    fn can_spawn_on(self, state: &BlockState, above_state: &BlockState) -> bool {
        match self {
            Self::OnTopOfCollider => {
                above_state.get_block_collision_shapes().next().is_none() && is_face_full_up(state)
            }
            Self::OnTopOfColliderNoLeaves => {
                above_state.get_block_collision_shapes().next().is_none()
                    && !Block::from_state_id(state.id).has_tag(&tag::Block::MINECRAFT_LEAVES)
                    && is_face_full_up(state)
            }
        }
    }
}

/// Vanilla `Block.isFaceFull(collisionShape, Direction.UP)`.
pub(crate) fn is_face_full_up(state: &BlockState) -> bool {
    state.is_full_cube()
        || state.get_block_collision_shapes().any(|shape| {
            shape.max.y >= 1.0
                && shape.min.x <= 0.0
                && shape.min.z <= 0.0
                && shape.max.x >= 1.0
                && shape.max.z >= 1.0
        })
}

#[expect(clippy::too_many_arguments)]
pub fn try_spawn_mob<T: Mob + 'static>(
    entity_type: &'static EntityType,
    create: fn(Entity) -> Arc<T>,
    world: &Arc<World>,
    start: &BlockPos,
    spawn_attempts: i32,
    spawn_range_xz: i32,
    spawn_range_y: i32,
    strategy: SpawnStrategy,
    check_collisions: bool,
) -> Option<Arc<T>> {
    for _ in 0..spawn_attempts {
        // `Mth.randomBetweenInclusive(level.getRandom(), -range, range)`, x then z.
        let (dx, dz) = {
            let mut random = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                random.next_bounded_i32(spawn_range_xz * 2 + 1) - spawn_range_xz,
                random.next_bounded_i32(spawn_range_xz * 2 + 1) - spawn_range_xz,
            )
        };
        let search_pos = start.add(dx, spawn_range_y, dz);
        let in_border = world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_block(search_pos.0.x, search_pos.0.z);
        if !in_border {
            continue;
        }
        let Some(spawn_pos) =
            move_to_possible_spawn_position(world, spawn_range_y, search_pos, strategy)
        else {
            continue;
        };

        let position = Vector3::new(
            f64::from(spawn_pos.0.x) + 0.5,
            f64::from(spawn_pos.0.y),
            f64::from(spawn_pos.0.z) + 0.5,
        );
        if check_collisions {
            let dimensions = EntityDimensions::new(
                entity_type.dimension[0],
                entity_type.dimension[1],
                entity_type.eye_height,
            );
            let bounding_box =
                BoundingBox::new_from_pos(position.x, position.y, position.z, &dimensions);
            if !world.no_collision(bounding_box) {
                continue;
            }
        }

        let mob = create(Entity::from_uuid(
            Uuid::new_v4(),
            world.clone(),
            position,
            entity_type,
        ));
        // `EntityType.create`: `snapTo(..., wrapDegrees(random.nextFloat() * 360), 0)`, then the
        // head and body take that yaw.
        let yaw = wrap_degrees(
            world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .next_f32()
                * 360.0,
        );
        let entity = mob.get_entity();
        entity.set_rotation(yaw, 0.0);
        entity.body_yaw.store(yaw);
        // `EntityType.create` finalizes the spawn.
        crate::entity::mob::spawn::finalize_spawn(
            &(mob.clone() as Arc<dyn EntityBase>),
            world,
            None,
        );
        if !mob.check_spawn_obstruction(world) {
            continue;
        }
        world.spawn_entity(mob.clone() as Arc<dyn EntityBase>);
        return Some(mob);
    }

    None
}

fn move_to_possible_spawn_position(
    world: &World,
    spawn_range_y: i32,
    mut search_pos: BlockPos,
    strategy: SpawnStrategy,
) -> Option<BlockPos> {
    let mut above_state = world.get_block_state(&search_pos);

    for _ in -spawn_range_y..=spawn_range_y {
        search_pos = search_pos.down();
        let current_state = world.get_block_state(&search_pos);
        if strategy.can_spawn_on(current_state, above_state) {
            return Some(search_pos.up());
        }
        above_state = current_state;
    }

    None
}
