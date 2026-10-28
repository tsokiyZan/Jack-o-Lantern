use crate::block::registry::BlockActionResult;
use std::sync::Arc;

use crate::block::registry::can_replace_with_other_block;
use crate::entity::decoration::armor_stand::ArmorStandEntity;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockDirection};
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::wrap_degrees;

pub struct ArmorStandItem;

impl ItemMetadata for ArmorStandItem {
    fn ids() -> Box<[u16]> {
        [Item::ARMOR_STAND.id].into()
    }
}

impl ItemBehaviour for ArmorStandItem {
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
        if face == BlockDirection::Down {
            return BlockActionResult::Fail;
        }

        let world = player.world();
        // Vanilla `BlockPlaceContext.getClickedPos`: the clicked block when it can be replaced.
        let (clicked_block, clicked_state) = world.get_block_and_state(&location);
        let target_pos = if can_replace_with_other_block(clicked_block, clicked_state) {
            location
        } else {
            location.offset(face.to_offset())
        };
        let bottom_center = Vector3::new(
            f64::from(target_pos.0.x) + 0.5,
            f64::from(target_pos.0.y),
            f64::from(target_pos.0.z) + 0.5,
        );

        let armor_stand_dimensions = EntityType::ARMOR_STAND.dimension;
        let width = f64::from(armor_stand_dimensions[0]);
        let height = f64::from(armor_stand_dimensions[1]);

        let bounding_box = BoundingBox::new(
            Vector3::new(
                bottom_center.x - width / 2.0,
                bottom_center.y,
                bottom_center.z - width / 2.0,
            ),
            Vector3::new(
                bottom_center.x + width / 2.0,
                bottom_center.y + height,
                bottom_center.z + width / 2.0,
            ),
        );

        // Vanilla `level.noCollision(null, box) && level.getEntities(null, box).isEmpty()`:
        // any entity but a spectator, players included, blocks the stand.
        if !world.no_collision(bounding_box)
            || world
                .get_all_at_box(&bounding_box)
                .iter()
                .any(|entity| !entity.is_spectator())
        {
            return BlockActionResult::Fail;
        }

        // Vanilla `EntityType.create(..., tryMoveDown = true, movedUp = true)`: the stand drops
        // onto the collision below, and the level random rolls a yaw the item then replaces.
        let entity_box = BoundingBox::new(
            Vector3::new(
                bottom_center.x - width / 2.0,
                bottom_center.y + 1.0,
                bottom_center.z - width / 2.0,
            ),
            Vector3::new(
                bottom_center.x + width / 2.0,
                bottom_center.y + 1.0 + height,
                bottom_center.z + width / 2.0,
            ),
        );
        let y_offset = crate::item::spawn_y_offset(&world, target_pos, true, &entity_box);
        let spawn_pos = Vector3::new(
            bottom_center.x,
            f64::from(target_pos.0.y) + y_offset,
            bottom_center.z,
        );
        let _ = world.random_f32();

        let (player_yaw, _) = player.rotation();
        let rotation = ((wrap_degrees(player_yaw - 180.0) + 22.5) / 45.0).floor() * 45.0;

        let entity = Entity::new(world.clone(), spawn_pos, &EntityType::ARMOR_STAND);
        entity.set_rotation(rotation, 0.0);
        let armor_stand: Arc<dyn EntityBase> = Arc::new(ArmorStandEntity::new(entity));
        world.spawn_entity(armor_stand.clone());

        let pos = armor_stand.get_entity().pos.load();
        world.play_sound_fine(
            Sound::EntityArmorStandPlace,
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
