#[allow(clippy::wildcard_imports)]
use super::*;
use crate::world::entity_tracker::encode_pos;
use pumpkin_data::entity::EntityPose;

impl BedrockClient {
    #[expect(clippy::too_many_lines)]
    pub fn handle_player_auth_input(
        &self,
        player: &Arc<Player>,
        packet: SPlayerAuthInput,
        server: &Arc<Server>,
    ) {
        if !player.has_client_loaded() {
            return;
        }
        if player.living_entity.dead.load(Ordering::Relaxed)
            || player.living_entity.health.load() <= 0.0
        {
            return;
        }
        let entity = player.get_entity();
        let on_ground = packet.input_data.get(InputData::VerticalCollision as usize)
            && packet.delta.y < 0.0
            && !entity.has_vehicle();
        entity.on_ground.store(on_ground, Ordering::Relaxed);

        let new_pos = packet
            .position
            .add_raw(0.0, -entity.entity_type.eye_height, 0.0)
            .to_f64();
        let old_pos = player.position();

        let new_pitch = packet.pitch;
        let new_yaw = packet.yaw;

        let old_pitch = entity.pitch.load();
        let old_yaw = entity.yaw.load();

        let pos_changed = new_pos != old_pos;
        let rot_changed = new_pitch != old_pitch || new_yaw != old_yaw;

        if pos_changed || rot_changed {
            let world = player.world();

            if pos_changed {
                player.get_entity().set_pos(new_pos);
            }
            if rot_changed {
                entity.pitch.store(new_pitch);
                entity.yaw.store(new_yaw);
            }

            // TODO: use `pumpkin_util::math::pack_degrees`.
            let je_yaw = (new_yaw * 256.0 / 360.0).rem_euclid(256.0);
            let je_pitch = (new_pitch * 256.0 / 360.0).rem_euclid(256.0);

            // Vanilla `ServerEntity.sendChanges`: a relative move while every encoded delta
            // fits in a short.
            let fits = |a: f64, b: f64| i16::try_from(encode_pos(a) - encode_pos(b)).is_ok();
            let fits_short = fits(new_pos.x, old_pos.x)
                && fits(new_pos.y, old_pos.y)
                && fits(new_pos.z, old_pos.z);

            let bedrock_move_packet = pumpkin_protocol::bedrock::client::CMovePlayer::new(
                pumpkin_protocol::codec::var_ulong::VarULong(player.entity_id() as u64),
                pumpkin_util::math::vector3::Vector3::new(
                    new_pos.x as f32,
                    new_pos.y as f32 + entity.entity_type.eye_height,
                    new_pos.z as f32,
                ),
                new_pitch,
                new_yaw,
                new_yaw, // Head yaw
                pumpkin_protocol::bedrock::client::CMovePlayer::MODE_NORMAL,
                on_ground,
                pumpkin_protocol::codec::var_ulong::VarULong(0),
                0,
                0,
                pumpkin_protocol::codec::var_ulong::VarULong(0),
            );

            if pos_changed && !fits_short {
                world.broadcast_packet_except(
                    &[player.gameprofile.id],
                    &pumpkin_protocol::java::client::play::CEntityPositionSync::new(
                        player.entity_id().into(),
                        new_pos,
                        pumpkin_util::math::vector3::Vector3::new(0.0, 0.0, 0.0),
                        je_yaw,
                        je_pitch,
                        on_ground,
                    ),
                );
            } else if pos_changed && rot_changed {
                world.broadcast_packet_except_editioned(
                    &[player.gameprofile.id],
                    &pumpkin_protocol::java::client::play::CUpdateEntityPosRot::new(
                        player.entity_id().into(),
                        pumpkin_util::math::vector3::Vector3::new(
                            (encode_pos(new_pos.x) - encode_pos(old_pos.x)) as i16,
                            (encode_pos(new_pos.y) - encode_pos(old_pos.y)) as i16,
                            (encode_pos(new_pos.z) - encode_pos(old_pos.z)) as i16,
                        ),
                        je_yaw as u8,   // Use converted Java byte
                        je_pitch as u8, // Use converted Java byte
                        on_ground,
                    ),
                    &bedrock_move_packet,
                );
            } else if pos_changed {
                world.broadcast_packet_except_editioned(
                    &[player.gameprofile.id],
                    &pumpkin_protocol::java::client::play::CUpdateEntityPos::new(
                        player.entity_id().into(),
                        pumpkin_util::math::vector3::Vector3::new(
                            (encode_pos(new_pos.x) - encode_pos(old_pos.x)) as i16,
                            (encode_pos(new_pos.y) - encode_pos(old_pos.y)) as i16,
                            (encode_pos(new_pos.z) - encode_pos(old_pos.z)) as i16,
                        ),
                        on_ground,
                    ),
                    &bedrock_move_packet,
                );
            } else if rot_changed {
                world.broadcast_packet_except_editioned(
                    &[player.gameprofile.id],
                    &pumpkin_protocol::java::client::play::CUpdateEntityRot::new(
                        player.entity_id().into(),
                        je_yaw as u8,   // Use converted Java byte
                        je_pitch as u8, // Use converted Java byte
                        on_ground,
                    ),
                    &bedrock_move_packet,
                );
            }

            if rot_changed {
                world.broadcast_packet_except(
                    &[player.gameprofile.id],
                    // Adjust to `CHeadRot` if that is what your crate currently calls it
                    &pumpkin_protocol::java::client::play::CHeadRot::new(
                        player.entity_id().into(),
                        je_yaw as u8,
                    ),
                );
            }

            if pos_changed {
                chunker::update_position(player);
                player.progress_motion(new_pos - old_pos);
            }
        }

        let input_data = packet.input_data;

        if input_data.get(InputData::StartSprinting as usize) {
            player.set_sprinting(true);
        } else if input_data.get(InputData::StopSprinting as usize) {
            player.set_sprinting(false);
        }

        if input_data.get(InputData::StartSneaking as usize) {
            entity.set_sneaking(true);
        } else if input_data.get(InputData::StopSneaking as usize) {
            entity.set_sneaking(false);
        }

        if input_data.get(InputData::StartCrawling as usize) {
            player.set_pose(EntityPose::Swimming);
        } else if input_data.get(InputData::StopCrawling as usize) {
            player.update_player_pose();
        }

        if input_data.get(InputData::StartFlying as usize) {
            let flying = {
                player
                    .abilities
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .flying
            };
            if !flying {
                send_cancellable_blocking! {{
                    server;
                    PlayerToggleFlightEvent::new(player.clone(), true);
                    'after: {
                        player.living_entity.entity.reset_fall_distance();
                        {
                            player.abilities.lock().unwrap_or_else(std::sync::PoisonError::into_inner).flying = true;
                        };
                        player.send_abilities_update();
                    }
                    'cancelled: {
                        player.send_abilities_update();
                    }
                }}
            }
        } else if input_data.get(InputData::StopFlying as usize) {
            let flying = {
                player
                    .abilities
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .flying
            };
            if flying {
                send_cancellable_blocking! {{
                    server;
                    PlayerToggleFlightEvent::new(player.clone(), false);
                    'after: {
                        {
                            player.abilities.lock().unwrap_or_else(std::sync::PoisonError::into_inner).flying = false;
                        };
                        player.send_abilities_update();
                    }
                    'cancelled: {
                        player.send_abilities_update();
                    }
                }}
            }
        }

        if let Some(block_actions) = packet.block_actions {
            for action in &block_actions {
                self.handle_player_block_action(player, server, action);
            }
        }
    }
}
