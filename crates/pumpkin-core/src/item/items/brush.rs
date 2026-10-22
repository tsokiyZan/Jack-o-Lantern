use std::any::Any;

use crate::block::entities::brushable_block::BrushableBlockBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::entity::Entity;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockDirection};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct BrushItem;

impl ItemMetadata for BrushItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::BRUSH.id])
    }
}

impl ItemBehaviour for BrushItem {
    /// Vanilla `BrushItem.useOn`: aimed at a block, the brush starts being used; brushing
    /// happens in `onUseTick`.
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        hand: pumpkin_util::Hand,
        _location: BlockPos,
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        _block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        if Self::hit_block(player).is_some() {
            crate::item::start_using_item(player, hand, item.clone(), Self::USE_DURATION);
        }
        BlockActionResult::Consume
    }

    /// Vanilla `BrushItem.onUseTick`: every tenth tick (the fifth of each stroke) the aimed
    /// block is brushed; aiming away stops the use.
    fn on_use_tick(&self, stack: &ItemStack, player: &Player, remaining_use_ticks: i32) {
        if remaining_use_ticks < 0 {
            player.living_entity.clear_active_hand();
            return;
        }
        let Some((pos, direction)) = Self::hit_block(player) else {
            player.living_entity.clear_active_hand();
            return;
        };
        let time_elapsed = Self::USE_DURATION - remaining_use_ticks + 1;
        if time_elapsed % 10 != 5 {
            return;
        }
        let world = player.world();
        let block = world.get_block(&pos);
        let sound = if block == &Block::SUSPICIOUS_SAND {
            Sound::ItemBrushBrushingSand
        } else if block == &Block::SUSPICIOUS_GRAVEL {
            Sound::ItemBrushBrushingGravel
        } else {
            Sound::ItemBrushBrushingGeneric
        };
        // `level.playSound(player, pos, ...)`: everyone near but the brushing player.
        world.play_sound_raw_expect(
            player,
            sound as u16,
            SoundCategory::Blocks,
            &pos.to_centered_f64(),
            1.0,
            1.0,
        );
        let Some(block_entity) = world.get_block_entity(&pos) else {
            return;
        };
        let Some(brushable) = block_entity
            .as_any()
            .downcast_ref::<BrushableBlockBlockEntity>()
        else {
            return;
        };
        if let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
            && let Some(server) = world.server.upgrade()
        {
            let mut event = crate::plugin::api::events::block::block_brush::BlockBrushEvent::new(
                pos,
                world.clone(),
                player_arc,
                stack.clone(),
            );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }
        if brushable.brush(&world, &player.living_entity, direction, stack) {
            // The brush wears in the hand it is used from.
            player.damage_item_in_hand(player.living_entity.used_item_hand(), 1);
        }
    }

    fn get_use_duration(&self, _stack: &ItemStack) -> i32 {
        Self::USE_DURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl BrushItem {
    pub const USE_DURATION: i32 = 200;

    /// Vanilla `calculateHitResult` when it hits a block: the block and the face, along the
    /// view within the block interaction range, unless a pickable entity is nearer
    /// (`ProjectileUtil.getHitResultOnViewVector` with `CAN_BE_PICKED`), which is no block.
    ///
    /// Vanilla's view vector is `getViewVector(0)`, the rotation at the start of the tick; a
    /// player's move packets snap that to the current rotation (`absSnapTo`), so the current
    /// rotation is the same.
    fn hit_block(player: &Player) -> Option<(BlockPos, BlockDirection)> {
        let (yaw, pitch) = player.rotation();
        let range = player
            .living_entity
            .get_attribute_value(&Attributes::BLOCK_INTERACTION_RANGE);
        let entity = &player.living_entity.entity;
        let delta = Entity::calculate_view_vector(pitch, yaw) * range;
        let from = player.eye_position();
        let mut to = from.add(&delta);
        let world = player.world();
        let hit = world.clip(&pumpkin_world::clip::ClipContext::new(
            from,
            to,
            pumpkin_world::clip::ClipBlock::Collider,
            pumpkin_world::clip::ClipFluid::None,
        ));
        if !hit.is_miss {
            to = hit.location;
        }
        // `getEntityHitResult` with no margin.
        let search = entity
            .bounding_box
            .load()
            .expand_towards(delta.x, delta.y, delta.z)
            .expand_all(1.0);
        let entity_hit = world.get_all_at_box(&search).iter().any(|other| {
            other.get_entity().entity_id != entity.entity_id
                && !other.is_spectator()
                && other.can_hit()
                && other
                    .get_entity()
                    .bounding_box
                    .load()
                    .clip(from, to)
                    .is_some()
        });
        if entity_hit || hit.is_miss {
            return None;
        }
        Some((hit.block_pos, hit.direction))
    }
}
