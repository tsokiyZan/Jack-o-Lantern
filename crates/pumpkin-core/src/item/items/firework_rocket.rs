use crate::block::registry::BlockActionResult;
use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::Arc;

use crate::entity::player::Player;
use crate::entity::projectile::firework_rocket::FireworkRocketEntity;
use crate::entity::{Entity, EntityBase, leashable};
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct FireworkRocketItem;

impl ItemMetadata for FireworkRocketItem {
    fn ids() -> Box<[u16]> {
        [Item::FIREWORK_ROCKET.id].into()
    }
}

impl ItemBehaviour for FireworkRocketItem {
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        _hand: pumpkin_util::Hand,
        location: BlockPos,
        _face: BlockDirection,
        cursor_pos: Vector3<f32>,
        _block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        let world = player.world();
        let entity = Entity::new(
            world.clone(),
            Vector3::new(
                f64::from(location.0.x) + f64::from(cursor_pos.x),
                f64::from(location.0.y) + f64::from(cursor_pos.y),
                f64::from(location.0.z) + f64::from(cursor_pos.z),
            ),
            &EntityType::FIREWORK_ROCKET,
        );
        let entity = FireworkRocketEntity::new(entity);
        world.spawn_entity(Arc::new(entity));
        item.decrement_unless_creative(player.gamemode.load(), 1);
        BlockActionResult::Success
    }

    /// Vanilla `FireworkRocketItem.use`.
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        if player.get_entity().is_fall_flying() {
            let world = player.world();
            if drop_all_leash_connections(player) {
                world.play_sound_from_entity(
                    player.get_entity(),
                    Sound::ItemLeadBreak,
                    SoundCategory::Neutral,
                    1.0,
                    1.0,
                );
            }
            let entity = Entity::new(
                world.clone(),
                player.get_entity().pos.load(),
                &EntityType::FIREWORK_ROCKET,
            );
            let entity = FireworkRocketEntity::new_shot(entity, player.get_entity());
            world.spawn_entity(Arc::new(entity));

            crate::item::consume_in_hand(player, hand);
            player.award_item_used(&Item::FIREWORK_ROCKET);
            return InteractionResult::SUCCESS;
        }
        InteractionResult::PASS
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Vanilla `Entity.dropAllLeashConnections(null)` for a player, which is never leashed itself:
/// every leashable it holds drops its lead, then a `SHEAR` game event with no source.
fn drop_all_leash_connections(player: &Player) -> bool {
    let entity = player.get_entity();
    let leashables = leashable::leashable_leashed_to(entity);
    if leashables.is_empty() {
        return false;
    }
    for leashable in &leashables {
        leashable::drop_leash(leashable.as_ref(), true, true);
    }
    entity
        .world
        .load()
        .game_event_from(None, GameEvent::Shear, entity.pos.load());
    true
}
