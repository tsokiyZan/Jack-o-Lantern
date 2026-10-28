use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::block::registry::BlockActionResult;
use crate::entity::decoration::item_frame::ItemFrameEntity;
use crate::entity::decoration::painting::PaintingEntity;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use crate::world::World;
use pumpkin_data::data_component_impl::{Holder, PaintingVariantImpl};
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::{Block, BlockDirection};
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct HangingEntityItem;

impl ItemMetadata for HangingEntityItem {
    fn ids() -> Box<[u16]> {
        [
            Item::PAINTING.id,
            Item::ITEM_FRAME.id,
            Item::GLOW_ITEM_FRAME.id,
        ]
        .into()
    }
}

impl ItemBehaviour for HangingEntityItem {
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        _hand: pumpkin_util::Hand,
        location: BlockPos,
        face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        _block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        let world = player.world();
        let target_pos = location.offset(face.to_offset());

        let spawned: Arc<dyn EntityBase> = if item.item.id == Item::PAINTING.id {
            // `HangingEntityItem.mayPlace`.
            if face == BlockDirection::Up || face == BlockDirection::Down {
                return BlockActionResult::Fail;
            }

            let pos = target_pos.to_centered_f64();
            // Vanilla `Painting.create`: the painting exists first and draws its variant from
            // its own random, also when the item's variant component replaces it afterwards.
            // Nothing fitting is a `CONSUME`.
            let entity = Entity::new(world.clone(), pos, &EntityType::PAINTING);
            let Some(random_variant) =
                PaintingEntity::choose_variant(&world, &entity, location, face)
            else {
                return BlockActionResult::Consume;
            };
            let variant = if let Some(comp) = item.get_data_component::<PaintingVariantImpl>() {
                // TODO: place inline (non-registry) paintings, which vanilla also accepts.
                let Holder::Reference(v) = comp.variant else {
                    return BlockActionResult::Fail;
                };
                // `survives()` with the item's variant.
                if !PaintingEntity::painting_fits(&world, location, face, v) {
                    return BlockActionResult::Consume;
                }
                v
            } else {
                random_variant
            };

            let spawn_pos = PaintingEntity::calculate_center_pos(
                location,
                face,
                variant.width(),
                variant.height(),
            );
            // `canCoexist(false)`: no other painting, nor any hanging entity facing the same way.
            let half = |size: u32, axis_step: i32| {
                if axis_step == 0 {
                    f64::from(size) / 2.0
                } else {
                    0.0625 / 2.0
                }
            };
            let offset = face.to_offset();
            let half_width = (
                half(variant.width(), offset.x),
                f64::from(variant.height()) / 2.0,
                half(variant.width(), offset.z),
            );
            if !can_coexist(
                &world,
                spawn_pos,
                half_width,
                face,
                &EntityType::PAINTING,
                false,
            ) {
                return BlockActionResult::Consume;
            }
            entity.set_pos(spawn_pos);
            entity
                .data
                .store(i32::from(face.to_index()), Ordering::Relaxed);
            Arc::new(PaintingEntity::new_with_variant(entity, variant))
        } else {
            // `ItemFrameItem.mayPlace`.
            if !world.is_in_build_limit(target_pos) {
                return BlockActionResult::Fail;
            }
            let entity_type = if item.item.id == Item::GLOW_ITEM_FRAME.id {
                &EntityType::GLOW_ITEM_FRAME
            } else {
                &EntityType::ITEM_FRAME
            };

            // Vanilla `ItemFrame.createBoundingBox`: the frame sits against the wall, its centre
            // 0.46875 back from the block centre, 0.75 wide and 0.0625 deep.
            let offset = face.to_offset();
            let center = target_pos.to_centered_f64();
            let pos = Vector3::new(
                center.x - f64::from(offset.x) * 0.46875,
                center.y - f64::from(offset.y) * 0.46875,
                center.z - f64::from(offset.z) * 0.46875,
            );
            let half = |axis_step: i32| if axis_step == 0 { 0.375 } else { 0.03125 };
            let half_width = (half(offset.x), half(offset.y), half(offset.z));
            if !item_frame_survives(&world, location, face, pos, half_width, entity_type) {
                return BlockActionResult::Consume;
            }

            let entity = Entity::new(world.clone(), pos, entity_type);
            let frame = ItemFrameEntity::new(entity);
            frame.set_facing(face);
            Arc::new(frame)
        };

        // `playPlacementSound`, then `ENTITY_PLACE` at the entity, then `addFreshEntity`.
        let pos = spawned.get_entity().pos.load();
        let sound = spawned
            .cast_any()
            .downcast_ref::<ItemFrameEntity>()
            .map_or(Sound::EntityPaintingPlace, ItemFrameEntity::get_place_sound);
        spawned.play_sound_at_self(sound, 1.0, 1.0);
        world.game_event_from(Some(player), GameEvent::EntityPlace, pos);
        world.spawn_entity(spawned);
        item.decrement_unless_creative(player.gamemode.load(), 1);
        BlockActionResult::Success
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Vanilla `ItemFrame.survives` for a new, unfixed frame: its pop box clear of blocks, a solid
/// block (or a diode, on a wall) behind it, and `canCoexist(true)`.
fn item_frame_survives(
    world: &World,
    location: BlockPos,
    face: BlockDirection,
    pos: Vector3<f64>,
    half_width: (f64, f64, f64),
    entity_type: &'static EntityType,
) -> bool {
    let pop_box = box_of_size(pos, half_width);
    if !world.is_space_empty(pop_box) {
        return false;
    }
    let (support_block, support_state) = world.get_block_and_state(&location);
    let supported = support_state.is_solid()
        || (face.is_horizontal()
            && (support_block == &Block::REPEATER || support_block == &Block::COMPARATOR));
    supported && can_coexist(world, pos, half_width, face, entity_type, true)
}

/// Vanilla `HangingEntity.canCoexist`: no other hanging entity in the pop box that faces the
/// same way, or, unless `allow_intersecting_same_type`, is of the same type.
fn can_coexist(
    world: &World,
    pos: Vector3<f64>,
    half_width: (f64, f64, f64),
    face: BlockDirection,
    entity_type: &'static EntityType,
    allow_intersecting_same_type: bool,
) -> bool {
    let pop_box = box_of_size(pos, half_width);
    !world.get_entities_at_box(&pop_box).iter().any(|other| {
        let other = other.get_entity();
        let is_hanging = [
            &EntityType::PAINTING,
            &EntityType::ITEM_FRAME,
            &EntityType::GLOW_ITEM_FRAME,
        ]
        .iter()
        .any(|t| t.id == other.entity_type.id);
        // Paintings and item frames keep their facing in the entity data.
        let same_direction = other.data.load(Ordering::Relaxed) == i32::from(face.to_index());
        is_hanging
            && ((!allow_intersecting_same_type && other.entity_type.id == entity_type.id)
                || same_direction)
    })
}

/// Vanilla `AABB.ofSize`.
fn box_of_size(center: Vector3<f64>, half_width: (f64, f64, f64)) -> BoundingBox {
    BoundingBox::new(
        Vector3::new(
            center.x - half_width.0,
            center.y - half_width.1,
            center.z - half_width.2,
        ),
        Vector3::new(
            center.x + half_width.0,
            center.y + half_width.1,
            center.z + half_width.2,
        ),
    )
}
