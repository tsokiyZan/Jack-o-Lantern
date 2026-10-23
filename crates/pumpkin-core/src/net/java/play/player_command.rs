#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_player_command(
        &self,
        player: &Arc<Player>,
        command: &SPlayerCommand,
        server: &Arc<Server>,
    ) {
        if command.entity_id != player.entity_id().into() {
            return;
        }
        if !player.has_client_loaded() {
            return;
        }
        player.update_last_action_time();

        let entity = &player.get_entity();
        match command.action {
            Action::StartSprinting => {
                if !entity.is_sprinting() {
                    send_cancellable_blocking! {{
                        server;
                        PlayerToggleSprintEvent::new(player.clone(), true);
                        'after: {
                            player.set_sprinting(event.is_sprinting);
                            player.update_player_pose();
                        }
                    }}
                }
            }
            Action::StopSprinting => {
                if entity.is_sprinting() {
                    send_cancellable_blocking! {{
                        server;
                        PlayerToggleSprintEvent::new(player.clone(), false);
                        'after: {
                            player.set_sprinting(event.is_sprinting);
                            player.update_player_pose();
                        }
                    }}
                }
            }
            Action::LeaveBed => {
                if player.is_sleeping() {
                    player.stop_sleep_in_bed(false, true);
                }
            }

            Action::StartHorseJump => {
                if let Some(vehicle) = player.get_entity().get_controlled_vehicle()
                    && let Some(mob) = vehicle.get_mob()
                    && mob.can_jump()
                    && command.jump_boost.0 > 0
                {
                    mob.handle_start_jump(command.jump_boost.0);
                }
            }
            Action::StopHorseJump => {
                if let Some(vehicle) = player.get_entity().get_controlled_vehicle()
                    && let Some(mob) = vehicle.get_mob()
                {
                    mob.handle_stop_jump();
                }
            }
            Action::OpenVehicleInventory => {
                if let Some(vehicle) = entity.get_vehicle()
                    && let Some(mob) = vehicle.get_mob()
                {
                    mob.open_custom_inventory_screen(player);
                }
            }
            Action::StartFlyingElytra => {
                let fall_flying = entity.check_fall_flying();
                if entity.is_fall_flying() != fall_flying {
                    let mut event = crate::plugin::api::events::entity::entity_toggle_glide::EntityToggleGlideEvent::new(
                        entity.entity_id,
                        fall_flying,
                    );
                    server.plugin_manager.fire_blocking(server, &mut event);
                    if !event.cancelled {
                        entity.set_fall_flying(event.is_gliding);
                    }
                }
            }
            // <= 1.21.5
            Action::StartSneaking | Action::StopSneaking => {
                self.handle_player_input(
                    player,
                    &SPlayerInput {
                        input: SPlayerInput::SNEAK,
                    },
                    server,
                );
            }
        }
    }
}
