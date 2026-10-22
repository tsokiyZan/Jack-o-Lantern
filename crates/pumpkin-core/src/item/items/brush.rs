use std::any::Any;
use std::sync::Arc;

use crate::block::entities::brushable_block::BrushableBlockBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::entity::Entity;
use crate::entity::item::ItemEntity;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use crate::world::World;
use pumpkin_data::block_properties::SuspiciousSandLikeProperties;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockDirection, BlockId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

pub struct BrushItem;

impl ItemMetadata for BrushItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::BRUSH.id])
    }
}
/// Vanilla `BrushableBlockEntity.unpackLootTable`, for the brushable block at `location`.
fn unpack_loot_table(world: &World, location: &BlockPos, player: &Player, brush: &ItemStack) {
    if let Some(block_entity) = world.get_block_entity(location)
        && let Some(brushable) = block_entity
            .as_any()
            .downcast_ref::<BrushableBlockBlockEntity>()
    {
        brushable.unpack_loot_table(world, &player.living_entity, brush);
    }
}

/// The item `dropContent` drops: the unpacked loot, if any.
fn take_brushed_item(world: &World, location: &BlockPos) -> Option<ItemStack> {
    let block_entity = world.get_block_entity(location)?;
    let brushable = block_entity
        .as_any()
        .downcast_ref::<BrushableBlockBlockEntity>()?;
    brushable
        .item
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .filter(|item| !item.is_empty())
}

impl ItemBehaviour for BrushItem {
    fn normal_use(&self, _item: &Item, player: &Player) {
        player.world().play_sound(
            Sound::ItemBrushBrushingGeneric,
            SoundCategory::Players,
            &player.position(),
        );
        let stack = player.inventory().held_item();
        player
            .living_entity
            .set_active_hand(pumpkin_util::Hand::Right, stack, Self::USE_DURATION);
    }

    fn use_on_block(
        &self,
        _item: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        let world = player.world();
        let is_sand = block.id == BlockId::SUSPICIOUS_SAND;
        let is_gravel = block.id == BlockId::SUSPICIOUS_GRAVEL;
        let block_center = location.to_centered_f64();

        if !(is_sand || is_gravel) {
            world.play_sound(
                Sound::ItemBrushBrushingGeneric,
                SoundCategory::Blocks,
                &block_center,
            );
            let stack = player.inventory().held_item();
            player.living_entity.set_active_hand(
                pumpkin_util::Hand::Right,
                stack,
                Self::USE_DURATION,
            );
            return BlockActionResult::Success;
        }

        if let Some(player_arc) = player.world().get_player_by_uuid(player.gameprofile.id)
            && let Some(server) = player.world().server.upgrade()
        {
            let mut event = crate::plugin::api::events::block::block_brush::BlockBrushEvent::new(
                location,
                world.clone(),
                player_arc,
                player.inventory().held_item(),
            );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return BlockActionResult::Fail;
            }
        }

        // `BrushableBlockEntity.brush` unpacks the loot table on every brush.
        let brush = player.inventory().held_item();
        unpack_loot_table(&world, &location, player, &brush);

        let current_state_id = world.get_block_state_id(&location);
        let mut props = SuspiciousSandLikeProperties::from_state_id(current_state_id);

        if props.dusted < 3 {
            props.dusted += 1;
            let next_stage_id = props.to_state_id(block);
            world.set_block_state(&location, next_stage_id, BlockFlags::NOTIFY_ALL);

            world.play_sound(
                if is_sand {
                    Sound::ItemBrushBrushingSand
                } else {
                    Sound::ItemBrushBrushingGravel
                },
                SoundCategory::Blocks,
                &block_center,
            );
        } else {
            // `brushingCompleted` drops the content before the block turns into sand or gravel.
            let loot_item = take_brushed_item(&world, &location);
            let replacement_state_id = if is_sand {
                Block::SAND.default_state.id
            } else {
                Block::GRAVEL.default_state.id
            };

            world.set_block_state(&location, replacement_state_id, BlockFlags::NOTIFY_ALL);

            world.play_sound(
                if is_sand {
                    Sound::ItemBrushBrushingSandComplete
                } else {
                    Sound::ItemBrushBrushingGravelComplete
                },
                SoundCategory::Blocks,
                &block_center,
            );

            if let Some(loot_item) = loot_item {
                let spawn_pos = Vector3::new(
                    f64::from(location.0.x) + 0.5,
                    f64::from(location.0.y) + 1.0,
                    f64::from(location.0.z) + 0.5,
                );
                let item_entity = Arc::new(ItemEntity::new(
                    Entity::new(world.clone(), spawn_pos, &EntityType::ITEM),
                    loot_item,
                ));
                world.spawn_entity(item_entity);
            }
        }

        player.damage_held_item(1);

        let stack = player.inventory().held_item();
        player
            .living_entity
            .set_active_hand(pumpkin_util::Hand::Right, stack, Self::USE_DURATION);

        BlockActionResult::Success
    }

    fn get_use_duration(&self) -> i32 {
        Self::USE_DURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl BrushItem {
    pub const USE_DURATION: i32 = 200;
}
