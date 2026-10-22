#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    #[expect(clippy::too_many_lines)]
    pub fn handle_interact(
        &self,
        player: &Arc<Player>,
        interact: &SInteract,
        server: &Arc<Server>,
    ) {
        if !player.has_client_loaded() {
            return;
        }
        player.update_last_action_time();
        let entity_id = interact.entity_id;

        let sneaking = interact.sneaking;
        let player_entity = &player.get_entity();
        if player_entity.is_sneaking() != sneaking {
            player_entity.set_sneaking(sneaking);
        }
        let Ok(action) = ActionType::try_from(interact.r#type.0) else {
            self.try_kick(&TextComponent::text("Invalid action type"));
            return;
        };

        // Resolve the target entity for the event
        let world = player_entity.world.load_full();
        let player_target = world.get_player_by_id(entity_id.0);
        let target: Option<Arc<dyn EntityBase>> = player_target
            .as_ref()
            .map(|p| Arc::clone(p) as Arc<dyn EntityBase>)
            .or_else(|| world.get_entity_or_part(entity_id.0));

        if let Some(target) = target {
            if player.gamemode.load() == GameMode::Spectator {
                player.camera_target_id.store(Some(entity_id.0));
                player.try_send_client_packet(&CSetCamera::new(entity_id));
                return;
            }
            send_cancellable_blocking! {{
                server;
                PlayerInteractEntityEvent::new(
                    player,
                    Arc::clone(&target),
                    action,
                    interact.target_position,
                    sneaking,
                );

                'after: {
                    match event.action {
                        ActionType::Attack => {
                            let config = &server.advanced_config.pvp;
                            if !config.enabled {
                                return;
                            }
                            // Vanilla `handleAttack` ignores the packet while the main hand jabs instead.
                            if player
                                .inventory()
                                .held_item()
                                .get_data_component::<pumpkin_data::data_component_impl::PiercingWeaponImpl>()
                                .is_some()
                            {
                                return;
                            }

                            if entity_id.0 == player.entity_id() {
                                self.try_kick(&TextComponent::translate_cross(translation::java::MULTIPLAYER_DISCONNECT_INVALID_ENTITY_ATTACKED, translation::java::MULTIPLAYER_DISCONNECT_INVALID_ENTITY_ATTACKED, []));
                                return;
                            }

                            if let Some(player_victim) = &player_target {
                                if player_victim.living_entity.health.load() <= 0.0 {
                                    return;
                                }
                                if config.protect_creative
                                    && player_victim.gamemode.load() == GameMode::Creative
                                {
                                    world
                                        .play_sound(
                                            Sound::EntityPlayerAttackNodamage,
                                            SoundCategory::Players,
                                            &player_victim.position(),
                                        )
                                        ;
                                    return;
                                }
                            }
                            if player.cannot_attack_with_item(&player.inventory().held_item(), 5) {
                                return;
                            }
                            player.attack(&event.target);
                        }
                        ActionType::Interact | ActionType::InteractAt => {
                            if event.action == ActionType::InteractAt
                                && let Some(pos) = interact.target_position
                            {
                                let mut at_event = crate::plugin::api::events::player::player_interact_at_entity::PlayerInteractAtEntityEvent::new(
                                    player.clone(),
                                    entity_id.0,
                                    pos.x,
                                    pos.y,
                                    pos.z,
                                    u8::from(interact.hand.map_or(0, |h| h.0) != 0),
                                );
                                server.plugin_manager.fire_blocking(server, &mut at_event);
                                if at_event.cancelled {
                                    return;
                                }
                            }
                            let mut stack = player.inventory().held_item();

                            let item_id = stack.item.id;
                            let before = stack.clone();
                            // Vanilla `Player.interactOn`: the entity's own `interact` first
                            // (for a mob, `Mob.interact`), then the item's
                            // `interactLivingEntity`. A mob already had the spawn egg.
                            let interacted = event.target.interact(player, &mut stack);
                            let is_mob_spawn_egg = event.target.get_mob().is_some()
                                && pumpkin_data::entity::entity_from_egg(stack.item.id).is_some();
                            if !interacted
                                && !stack.is_empty()
                                && !is_mob_spawn_egg
                                && server.item_registry.use_on_entity(
                                    &mut stack,
                                    player,
                                    Arc::clone(&event.target),
                                )
                                && event.target.get_living_entity().is_some()
                            {
                                world.game_event_from(
                                    Some(player.as_ref()),
                                    pumpkin_data::game_event::GameEvent::EntityInteract,
                                    event.target.get_entity().pos.load(),
                                );
                            }
                            if !stack.are_equal(&before) && before.is_damageable() && stack.is_empty() {
                                player.increment_stat(
                                    StatisticCategory::Broken,
                                    item_id as i32,
                                    1,
                                );
                                player.world().send_entity_status(
                                    player.get_entity(),
                                    equipment_break_status(&EquipmentSlot::MAIN_HAND),
                                    None,
                                );
                            }
                            player.inventory().set_held_item(stack);
                        }
                    }
                }
            }}
        } else {
            // Entity not found
            send_cancellable_blocking! {{
                server;
                PlayerInteractUnknownEntityEvent::new(player, entity_id.0, action);

                'after: {
                    if event.action == ActionType::Attack {
                        error!(
                            "Player id {} interacted with entity id {}, which was not found.",
                            player.entity_id(),
                            event.entity_id
                        );
                        self.try_kick(&TextComponent::translate_cross(translation::java::MULTIPLAYER_DISCONNECT_INVALID_ENTITY_ATTACKED, translation::java::MULTIPLAYER_DISCONNECT_INVALID_ENTITY_ATTACKED, []));
                    }
                }
            }}
        }
    }
}
