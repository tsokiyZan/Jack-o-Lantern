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
    is_face_full_up_shapes(state.get_block_collision_shapes())
}

/// Vanilla `Block.isFaceFull(shape, Direction.UP)`: the boxes of `getFaceShape(UP)`, the
/// layer just below the top, together cover the whole face.
pub(crate) fn is_face_full_up_shapes(shapes: impl IntoIterator<Item = BoundingBox>) -> bool {
    const SLICE_Y: f64 = 1.0 - 1.0e-7;
    let rects: Vec<[f64; 4]> = shapes
        .into_iter()
        .filter(|shape| shape.min.y <= SLICE_Y && shape.max.y > SLICE_Y)
        .map(|shape| [shape.min.x, shape.min.z, shape.max.x, shape.max.z])
        .collect();
    let edges = |lo: usize, hi: usize| {
        let mut edges: Vec<f64> = rects
            .iter()
            .flat_map(|rect| [rect[lo], rect[hi]])
            .filter(|edge| (0.0..=1.0).contains(edge))
            .chain([0.0, 1.0])
            .collect();
        edges.sort_by(f64::total_cmp);
        edges.dedup();
        edges
    };
    let (xs, zs) = (edges(0, 2), edges(1, 3));
    // Every cell between the box edges has to lie inside some box.
    xs.windows(2).all(|x| {
        zs.windows(2).all(|z| {
            let (mx, mz) = (f64::midpoint(x[0], x[1]), f64::midpoint(z[0], z[1]));
            rects
                .iter()
                .any(|rect| rect[0] <= mx && mx <= rect[2] && rect[1] <= mz && mz <= rect[3])
        })
    })
}

/// Vanilla `EntityCollisionContext.isAbove`: the entity's feet are above `shape_max_y` of the
/// block at `pos`.
pub(crate) fn is_above(entity: &Entity, pos: &BlockPos, shape_max_y: f64) -> bool {
    entity.pos.load().y > f64::from(pos.0.y) + shape_max_y - f64::from(1.0e-5f32)
}

/// Vanilla `BlockState.entityCanStandOn`: the top face of the block's collision shape for
/// `entity` is full. Powder snow and scaffolding are the blocks whose shape depends on it.
pub(crate) fn entity_can_stand_on(world: &World, pos: &BlockPos, entity: &dyn EntityBase) -> bool {
    let state = world.get_block_state(pos);
    let block = Block::from_state_id(state.id);
    if block == &Block::POWDER_SNOW {
        return crate::block::blocks::powder_snow::collision_shape_for_entity(entity, pos)
            .is_some_and(|shape| is_face_full_up_shapes([shape]));
    }
    if block == &Block::SCAFFOLDING {
        // `SHAPE_STABLE` (full top) when above and not descending; otherwise the unstable
        // bottom plate or nothing, neither with a full top.
        let base = entity.get_entity();
        return is_above(base, pos, 1.0)
            && !crate::block::blocks::powder_snow::is_entity_descending(base);
    }
    is_face_full_up(state)
}

#[expect(clippy::too_many_arguments)]
pub fn try_spawn_mob<T: Mob + 'static>(
    entity_type: &'static EntityType,
    create: fn(Entity) -> Arc<T>,
    reason: crate::entity::mob::spawn::EntitySpawnReason,
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
            reason,
            None,
        );
        // `checkSpawnRules` (`PathfinderMob`'s walk target value) and `checkSpawnObstruction`.
        let spawn_pos = BlockPos::floored_v(position);
        if mob.get_walk_target_value(&spawn_pos) < 0.0 || !mob.check_spawn_obstruction(world) {
            continue;
        }
        world.spawn_entity(mob.clone() as Arc<dyn EntityBase>);
        mob.play_ambient_sound();
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

#[cfg(test)]
mod tests {
    use super::is_face_full_up_shapes;
    use pumpkin_util::math::boundingbox::BoundingBox;

    fn shape(min: [f64; 3], max: [f64; 3]) -> BoundingBox {
        BoundingBox::new_array(min, max)
    }

    #[test]
    fn face_full_up_is_the_union_of_the_top_layer() {
        // Stone, a top slab and upside-down stairs are full on top; a bottom slab, a fence
        // post and half a face are not. Two halves together cover it, as `isFaceFull` does.
        assert!(is_face_full_up_shapes([shape([0.0; 3], [1.0; 3])]));
        assert!(is_face_full_up_shapes([shape([0.0, 0.5, 0.0], [1.0; 3])]));
        assert!(is_face_full_up_shapes([
            shape([0.0, 0.5, 0.0], [1.0; 3]),
            shape([0.0, 0.0, 0.0], [1.0, 0.5, 0.5]),
        ]));
        assert!(!is_face_full_up_shapes([shape([0.0; 3], [1.0, 0.5, 1.0])]));
        assert!(!is_face_full_up_shapes([shape(
            [0.375, 0.0, 0.375],
            [0.625, 1.5, 0.625]
        )]));
        assert!(!is_face_full_up_shapes([shape([0.0; 3], [0.5, 1.0, 1.0])]));
        assert!(is_face_full_up_shapes([
            shape([0.0; 3], [0.5, 1.0, 1.0]),
            shape([0.5, 0.0, 0.0], [1.0; 3]),
        ]));
    }
}
