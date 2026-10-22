#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_data::entity::EntityType;

impl JavaClient {
    /// Vanilla `ServerGamePacketListenerImpl.handleAttack`.
    pub fn handle_attack(&self, player: &Arc<Player>, attack: &SAttack, server: &Arc<Server>) {
        if !player.has_client_loaded() || player.gamemode.load() == GameMode::Spectator {
            return;
        }
        player.update_last_action_time();
        let world = player.get_entity().world.load_full();
        let target: Option<Arc<dyn EntityBase>> = world
            .get_player_by_id(attack.entity_id.0)
            .map(|p| p as Arc<dyn EntityBase>)
            .or_else(|| world.get_entity_or_part(attack.entity_id.0));
        // `getEntityOrPart`: an unknown target is ignored.
        let Some(target) = target else {
            return;
        };
        self.attack_target(player, &target, server);
    }

    /// The rest of vanilla `handleAttack` once the target is known: the world border, the
    /// main-hand weapon's attack range, a jab instead of an attack, the targets a client never
    /// attacks (kicked), and the attack charge.
    pub(super) fn attack_target(
        &self,
        player: &Arc<Player>,
        target: &Arc<dyn EntityBase>,
        server: &Arc<Server>,
    ) {
        let world = player.get_entity().world.load_full();
        let target_entity = target.get_entity();
        let block_pos = target_entity.block_pos.load();
        let in_border = world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_block(block_pos.0.x, block_pos.0.z);
        if !in_border {
            return;
        }
        let main_hand = player.inventory().held_item();
        if !player.is_within_attack_range(&main_hand, &target_entity.bounding_box.load(), 3.0) {
            return;
        }
        // The main hand jabs instead.
        if main_hand
            .get_data_component::<pumpkin_data::data_component_impl::PiercingWeaponImpl>()
            .is_some()
        {
            return;
        }
        let target_type = target_entity.entity_type;
        // `ItemEntity`, `ExperienceOrb`, the player itself, and an `AbstractArrow` that isn't
        // attackable.
        if target_type == &EntityType::ITEM
            || target_type == &EntityType::EXPERIENCE_ORB
            || target_entity.entity_id == player.entity_id()
            || ([
                &EntityType::ARROW,
                &EntityType::SPECTRAL_ARROW,
                &EntityType::TRIDENT,
            ]
            .contains(&target_type)
                && !target.is_attackable())
        {
            self.try_kick(&TextComponent::translate_cross(
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_ENTITY_ATTACKED,
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_ENTITY_ATTACKED,
                [],
            ));
            warn!(
                "Player {} tried to attack an invalid entity",
                player.gameprofile.name
            );
            return;
        }

        // Pumpkin's `pvp` config.
        let config = &server.advanced_config.pvp;
        if !config.enabled {
            return;
        }
        if let Some(player_victim) = target.get_player() {
            if player_victim.living_entity.health.load() <= 0.0 {
                return;
            }
            if config.protect_creative && player_victim.gamemode.load() == GameMode::Creative {
                world.play_sound(
                    Sound::EntityPlayerAttackNodamage,
                    SoundCategory::Players,
                    &player_victim.position(),
                );
                return;
            }
        }
        if player.cannot_attack_with_item(&main_hand, 5) {
            return;
        }
        player.attack(target);
    }
}
