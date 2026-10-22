#[allow(clippy::wildcard_imports)]
use super::*;
use crate::entity::player::cooldown_group;
use pumpkin_util::version::JavaMinecraftVersion;

impl JavaClient {
    pub fn handle_use_item(&self, player: &Arc<Player>, use_item: &SUseItem, server: &Arc<Server>) {
        if !player.has_client_loaded() {
            return;
        }
        player.update_last_action_time();

        let inventory = player.inventory();
        let Ok(hand) = Hand::from_packet_id(use_item.hand.0) else {
            self.try_kick(&TextComponent::text("InvalidHand"));
            return;
        };
        if self.version.load() >= JavaMinecraftVersion::V_1_21
            && (!use_item.yaw.is_finite() || !use_item.pitch.is_finite())
        {
            self.try_kick(&TextComponent::text("Invalid item use rotation"));
            return;
        }
        self.update_sequence(use_item.sequence.0);

        let item_in_hand = inventory.get_stack_in_hand(hand);
        if item_in_hand.is_empty() {
            return;
        }
        let swing_animation = item_in_hand.get_interact_animation();
        if self.version.load() >= JavaMinecraftVersion::V_1_21 {
            // `absSnapRotationTo` the packet's wrapped rotation.
            let (yaw, pitch) = (wrap_degrees(use_item.yaw), wrap_degrees(use_item.pitch));
            if (yaw, pitch) != player.rotation() {
                let entity = player.get_entity();
                entity.yaw.store(yaw % 360.0);
                entity.set_pitch(pitch);
            }
        }
        let (use_yaw, use_pitch) = player.rotation();
        // `ServerPlayerGameMode.useItem`: spectators and items on cooldown pass.
        if player.gamemode.load() == GameMode::Spectator
            || player.is_on_cooldown(&cooldown_group(&item_in_hand))
        {
            return;
        }

        let mut consume_event =
            crate::plugin::api::events::player::player_item_consume::PlayerItemConsumeEvent::new(
                player.clone(),
                item_in_hand.item.registry_key.to_string(),
            );
        server
            .plugin_manager
            .fire_blocking(server, &mut consume_event);
        if consume_event.cancelled {
            return;
        }

        // Vanilla's `ITEM_USED` is awarded by each item's `use`, not here.
        let hit = player.world().clip(&pumpkin_world::clip::ClipContext::new(
            player.eye_position(),
            player
                .eye_position()
                .add(&(Vector3::rotation_vector(f64::from(use_pitch), f64::from(use_yaw)) * 4.5)),
            pumpkin_world::clip::ClipBlock::Outline,
            pumpkin_world::clip::ClipFluid::None,
        ));

        let event = if hit.is_miss {
            PlayerInteractEvent::new(player, InteractAction::RightClickAir, &Block::AIR, None)
        } else {
            let hit_pos = hit.block_pos;
            PlayerInteractEvent::new(
                player,
                InteractAction::RightClickBlock,
                player.world().get_block(&hit_pos),
                Some(hit_pos),
            )
        };

        if !Self::should_continue_use_after_fish_event(server, player, hand, item_in_hand.item) {
            return;
        }

        send_cancellable_blocking! {{
            server;
            event;
            'after: {
                let result = server
                    .item_registry
                    .use_item(&item_in_hand, player, hand);
                // `handleUseItem`: a success swings and resets the attack strength.
                player.swing_for_interaction(hand, swing_animation, result.swing_source());
            }
        }}
    }

    fn should_continue_use_after_fish_event(
        server: &Arc<Server>,
        player: &Arc<Player>,
        hand: Hand,
        item_for_use: &Item,
    ) -> bool {
        if item_for_use.id != Item::FISHING_ROD.id {
            return true;
        }

        let mut fish_event = PlayerFishEvent::new(
            player.clone(),
            None,
            uuid::Uuid::nil(),
            String::new(),
            PlayerFishState::Fishing,
            hand,
            0,
        );
        server.plugin_manager.fire_blocking(server, &mut fish_event);
        !fish_event.cancelled
    }
}
