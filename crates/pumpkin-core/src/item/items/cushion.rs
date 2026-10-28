use crate::block::registry::BlockActionResult;
use std::sync::Arc;

use crate::block::registry::can_replace_with_other_block;
use crate::entity::decoration::cushion::CushionEntity;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use crate::world::World;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection};
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct CushionItem;

impl ItemMetadata for CushionItem {
    fn ids() -> Box<[u16]> {
        [
            Item::WHITE_CUSHION.id,
            Item::ORANGE_CUSHION.id,
            Item::MAGENTA_CUSHION.id,
            Item::LIGHT_BLUE_CUSHION.id,
            Item::YELLOW_CUSHION.id,
            Item::LIME_CUSHION.id,
            Item::PINK_CUSHION.id,
            Item::GRAY_CUSHION.id,
            Item::LIGHT_GRAY_CUSHION.id,
            Item::CYAN_CUSHION.id,
            Item::PURPLE_CUSHION.id,
            Item::BLUE_CUSHION.id,
            Item::BROWN_CUSHION.id,
            Item::GREEN_CUSHION.id,
            Item::RED_CUSHION.id,
            Item::BLACK_CUSHION.id,
        ]
        .into()
    }
}

impl ItemBehaviour for CushionItem {
    /// Vanilla `CushionItem.useOn`.
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        _hand: pumpkin_util::Hand,
        location: BlockPos,
        face: BlockDirection,
        cursor_pos: Vector3<f32>,
        _block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        let world = player.world();
        let click = Vector3::new(
            f64::from(location.0.x) + f64::from(cursor_pos.x),
            f64::from(location.0.y) + f64::from(cursor_pos.y),
            f64::from(location.0.z) + f64::from(cursor_pos.z),
        );
        let (face, click) = recalculate_for_special_collision_shapes(player, location, face, click);
        if face != BlockDirection::Up {
            return BlockActionResult::Fail;
        }

        // `BlockPlaceContext.getClickedPos`.
        let (clicked_block, clicked_state) = world.get_block_and_state(&location);
        let block_pos = if can_replace_with_other_block(clicked_block, clicked_state) {
            location
        } else {
            location.up()
        };
        let entity_pos = Vector3::new(
            f64::from(block_pos.0.x) + 0.5,
            click.y,
            f64::from(block_pos.0.z) + 0.5,
        );
        // `EntityType.getSpawnAABB`.
        let scale = EntityType::CUSHION.spawn_dimensions_scale;
        let half_width = f64::from(scale * EntityType::CUSHION.dimension[0] / 2.0);
        let height = f64::from(scale * EntityType::CUSHION.dimension[1]);
        let spawn_box = BoundingBox::new(
            Vector3::new(
                entity_pos.x - half_width,
                entity_pos.y,
                entity_pos.z - half_width,
            ),
            Vector3::new(
                entity_pos.x + half_width,
                entity_pos.y + height,
                entity_pos.z + half_width,
            ),
        );
        if !can_be_placed_at(&world, &spawn_box) {
            return BlockActionResult::Fail;
        }
        // Only other cushions block a cushion.
        if world
            .get_entities_at_box(&spawn_box)
            .iter()
            .any(|entity| entity.get_entity().entity_type.id == EntityType::CUSHION.id)
        {
            return BlockActionResult::Fail;
        }

        // `EntityType.create` rolls a yaw from the level random before `snapTo` replaces it
        // with the facing nearest the player's rotation.
        let _ = world.random_f32();
        let (player_yaw, _) = player.rotation();
        let yaw = ((f64::from(player_yaw) / 90.0 + 0.5).floor() as i32 & 3) as f32 * 90.0;
        let entity = Entity::new(world.clone(), entity_pos, &EntityType::CUSHION);
        entity.set_rotation(yaw, 0.0);
        let cushion: Arc<dyn EntityBase> = Arc::new(CushionEntity::new(
            entity,
            CushionEntity::color_from_item(item.get_item()),
        ));
        world.spawn_entity(cushion.clone());
        destroy_if_in_fire(&world, cushion.as_ref());

        let pos = cushion.get_entity().pos.load();
        world.play_sound_fine(
            Sound::EntityCushionPlace,
            SoundCategory::Blocks,
            &pos,
            0.75,
            0.8,
        );
        world.game_event_from(Some(player), GameEvent::EntityPlace, pos);
        item.decrement_unless_creative(player.gamemode.load(), 1);
        BlockActionResult::Success
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Vanilla `CushionItem.recalculateContextForSpecialCollisionShapes`: on blocks whose collision
/// shape differs from their outline, the click moves to where the ray meets the collision shape.
fn recalculate_for_special_collision_shapes(
    player: &Player,
    location: BlockPos,
    face: BlockDirection,
    click: Vector3<f64>,
) -> (BlockDirection, Vector3<f64>) {
    let world = player.world();
    let (block, state) = world.get_block_and_state(&location);
    if !block.has_tag(&tag::Block::MINECRAFT_CUSHION_USES_COLLISION_SHAPE) {
        return (face, click);
    }
    let ray_from = player.eye_position();
    let ray = click.sub(&ray_from);
    let length = ray.length();
    let ray_to = click.add(&Vector3::new(
        ray.x / length * COLLISION_SHAPE_RAYCAST_EPSILON,
        ray.y / length * COLLISION_SHAPE_RAYCAST_EPSILON,
        ray.z / length * COLLISION_SHAPE_RAYCAST_EPSILON,
    ));
    pumpkin_world::clip::clip_block_shapes(
        state.get_block_collision_shapes_at(&location),
        state,
        &location,
        ray_from,
        ray_to,
    )
    .map_or((face, click), |(_, direction, hit, _)| (direction, hit))
}

/// Vanilla `CushionItem.COLLISION_SHAPE_RAYCAST_EPSILON`.
const COLLISION_SHAPE_RAYCAST_EPSILON: f64 = 0.001;

/// Vanilla `Cushion.canBePlacedAt`.
fn can_be_placed_at(world: &World, aabb: &BoundingBox) -> bool {
    has_anchor_below(world, aabb)
        && !is_covered_by_suffocating_blocks(world, aabb)
        && !is_anchor_buried(world, aabb)
}

/// Vanilla `Cushion.hasAnchorBelow`: some block's outline bounds reach the 1/64 slice under
/// the cushion.
fn has_anchor_below(world: &World, aabb: &BoundingBox) -> bool {
    let anchor_box = BoundingBox::new(
        Vector3::new(aabb.min.x, aabb.min.y - 0.015_625, aabb.min.z),
        Vector3::new(aabb.max.x.next_down(), aabb.min.y, aabb.max.z.next_down()),
    );
    let search_min = Vector3::new(anchor_box.min.x, anchor_box.min.y - 0.125, anchor_box.min.z);
    BlockPos::iterate(
        BlockPos::floored_v(search_min),
        BlockPos::floored_v(anchor_box.max),
    )
    .any(|pos| {
        let state = world.get_block_state(&pos);
        let mut shapes = state.get_block_outline_shapes_at(&pos);
        let Some(first) = shapes.next() else {
            return false;
        };
        // `shape.bounds()`: the box around every part.
        let bounds = shapes.fold(first, |acc, shape| {
            BoundingBox::new(
                Vector3::new(
                    acc.min.x.min(shape.min.x),
                    acc.min.y.min(shape.min.y),
                    acc.min.z.min(shape.min.z),
                ),
                Vector3::new(
                    acc.max.x.max(shape.max.x),
                    acc.max.y.max(shape.max.y),
                    acc.max.z.max(shape.max.z),
                ),
            )
        });
        bounds.at_pos(pos).intersects(&anchor_box)
    })
}

/// Vanilla `AABB.nextDeflated`.
const fn next_deflated(aabb: &BoundingBox) -> BoundingBox {
    BoundingBox::new(
        Vector3::new(
            aabb.min.x.next_up(),
            aabb.min.y.next_up(),
            aabb.min.z.next_up(),
        ),
        Vector3::new(
            aabb.max.x.next_down(),
            aabb.max.y.next_down(),
            aabb.max.z.next_down(),
        ),
    )
}

/// Vanilla `Cushion.isCoveredBySuffocatingBlocks`.
fn is_covered_by_suffocating_blocks(world: &World, aabb: &BoundingBox) -> bool {
    let inner = next_deflated(aabb);
    BlockPos::iterate(
        BlockPos::floored_v(inner.min),
        BlockPos::floored_v(inner.max),
    )
    .all(|pos| world.get_block_state(&pos).is_suffocating())
}

/// Vanilla `Cushion.isAnchorBuried`: block collisions cover the whole 1/64 slice the cushion
/// rests on.
fn is_anchor_buried(world: &World, aabb: &BoundingBox) -> bool {
    let slice = next_deflated(&BoundingBox::new(
        aabb.min,
        Vector3::new(aabb.max.x, aabb.min.y + 0.015_625, aabb.max.z),
    ));
    let colliders: Vec<BoundingBox> =
        BlockPos::iterate(slice.min_block_pos(), slice.max_block_pos())
            .flat_map(|pos| {
                world
                    .get_block_state(&pos)
                    .get_block_collision_shapes_at(&pos)
                    .map(move |shape| shape.at_pos(pos))
                    .collect::<Vec<_>>()
            })
            .filter(|shape| shape.intersects(&slice))
            .collect();
    if colliders.is_empty() {
        return false;
    }
    // `Shapes.join(slice, colliders, ONLY_FIRST).isEmpty()`: every cell of the slice, split
    // at the colliders' faces, lies inside some collider.
    let cuts = |min: f64, max: f64, get: fn(&BoundingBox) -> [f64; 2]| {
        let mut points = vec![min, max];
        for collider in &colliders {
            points.extend(get(collider).into_iter().filter(|p| *p > min && *p < max));
        }
        points.sort_by(f64::total_cmp);
        points.dedup();
        points
    };
    let xs = cuts(slice.min.x, slice.max.x, |b| [b.min.x, b.max.x]);
    let ys = cuts(slice.min.y, slice.max.y, |b| [b.min.y, b.max.y]);
    let zs = cuts(slice.min.z, slice.max.z, |b| [b.min.z, b.max.z]);
    let mid = |points: &[f64], i: usize| f64::midpoint(points[i], points[i + 1]);
    (0..xs.len() - 1).all(|i| {
        (0..ys.len() - 1).all(|j| {
            (0..zs.len() - 1).all(|k| {
                let point = Vector3::new(mid(&xs, i), mid(&ys, j), mid(&zs, k));
                colliders.iter().any(|collider| collider.contains(point))
            })
        })
    })
}

/// Vanilla `Cushion.destroyIfInFire`: a fire block in its box hurts it once.
fn destroy_if_in_fire(world: &World, cushion: &dyn EntityBase) {
    let entity = cushion.get_entity();
    if entity.is_removed() {
        return;
    }
    let inner = next_deflated(&entity.bounding_box.load());
    if BlockPos::iterate(
        BlockPos::floored_v(inner.min),
        BlockPos::floored_v(inner.max),
    )
    .any(|pos| world.get_block(&pos).has_tag(&tag::Block::MINECRAFT_FIRE))
    {
        cushion.damage(cushion, 1.0, DamageType::IN_FIRE);
    }
}
