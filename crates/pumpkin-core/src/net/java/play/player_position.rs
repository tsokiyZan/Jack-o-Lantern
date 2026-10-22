#[allow(clippy::wildcard_imports)]
use super::*;
use crate::world::entity_tracker::encode_pos;

impl JavaClient {
    pub(super) const fn clamp_horizontal(pos: f64) -> f64 {
        pos.clamp(-3.0E7, 3.0E7)
    }

    pub(super) const fn clamp_vertical(pos: f64) -> f64 {
        pos.clamp(-2.0E7, 2.0E7)
    }

    /// Returns whether syncing the position was needed
    fn sync_position(
        player: &Arc<Player>,
        world: &World,
        entity: &Entity,
        pos: Vector3<f64>,
        last_pos: Vector3<f64>,
        on_ground: bool,
    ) -> bool {
        let entity_id = player.entity_id();

        // Vanilla `ServerEntity.sendChanges`: a relative move while every encoded delta
        // fits in a short, otherwise a position sync.
        let fits = |a: f64, b: f64| i16::try_from(encode_pos(a) - encode_pos(b)).is_ok();
        if fits(pos.x, last_pos.x) && fits(pos.y, last_pos.y) && fits(pos.z, last_pos.z) {
            return false;
        }
        // Sync position with tracking players only.
        world.send_to_tracking_players_editioned(
            entity,
            &CEntityPositionSync::new(
                entity_id.into(),
                pos,
                Vector3::new(0.0, 0.0, 0.0),
                entity.yaw.load(),
                entity.pitch.load(),
                on_ground,
            ),
            &bedrock_move_player_packet(entity, pos, CMovePlayer::MODE_TELEPORT, on_ground),
        );
        // Bedrock ignores head yaw on teleport -> follow-up normal move re-asserts.
        world.send_to_tracking_players_bedrock(
            entity,
            &bedrock_move_player_packet(entity, pos, CMovePlayer::MODE_NORMAL, on_ground),
        );
        true
    }

    /// The parts of vanilla's server-side `player.move` Pumpkin runs: step sounds and `step`
    /// events (`applyMovementEmissionAndPlaySound`) and the block speed factor, then
    /// `setOnGroundWithMovement`'s supporting block. Inside `move` the supporting block only
    /// updates when the player moved vertically, so flat walking steps on the block from the
    /// previous packet.
    fn apply_player_move(
        player: &Arc<Player>,
        last_pos: Vector3<f64>,
        pos: Vector3<f64>,
        on_ground: bool,
        horizontal_collision: bool,
    ) {
        let entity = player.get_entity();
        let caller: &dyn EntityBase = player.as_ref();
        let movement = pos.sub(&last_pos);
        // Spectators have `noPhysics`, and `move` returns before any of this.
        if !player.is_spectator() {
            entity.reset_fall_on_long_move(last_pos, movement);
            // `move` records the move for the block sweep in `applyEffectsFromBlocks`.
            entity.record_movement(last_pos, pos, movement);
            if movement.y != 0.0 {
                entity.check_supporting_block(caller, on_ground, Some(movement));
            }
            let effect_pos = entity.get_on_pos_legacy();
            let effect_state = player.world().get_block_state(&effect_pos);
            entity.apply_movement_emission_and_play_sound(
                caller,
                movement,
                effect_pos,
                effect_state,
            );
            Self::apply_move_speed_factor(player);
        }
        // `setOnGroundWithMovement` with the client's flags.
        entity
            .horizontal_collision
            .store(horizontal_collision, Ordering::Relaxed);
        entity.check_supporting_block(caller, on_ground, Some(movement));
    }

    /// Vanilla `handleMovePlayer`: anything that ends a fall ends the current impulse too, once
    /// its grace time is over.
    fn try_reset_impulse_after_move(player: &Player, on_ground: bool) {
        let entity = player.get_entity();
        // `hasLandedInLiquid`.
        let landed_in_liquid = entity.velocity.load().y < 1.0E-5 && entity.is_in_liquid();
        let on_climbable = player.living_entity.on_climbable(player);
        if on_ground
            || landed_in_liquid
            || on_climbable
            || player.is_spectator()
            || entity.is_fall_flying()
        {
            player.living_entity.try_reset_current_impulse_context();
        }
    }

    /// The end of vanilla's server-side `player.move`, which every movement packet runs unless
    /// the player waits on a teleport, rides, sleeps or is a spectator (`noPhysics`).
    pub(super) fn apply_move_speed_factor(player: &Player) {
        let entity = player.get_entity();
        if !player.has_client_loaded()
            || entity.has_vehicle()
            || player.is_sleeping()
            || player.is_spectator()
            || player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some()
        {
            return;
        }
        entity.apply_block_speed_factor(player);
    }

    #[expect(clippy::too_many_lines)]
    pub fn handle_position(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        packet: &SPlayerPosition,
    ) {
        if !player.has_client_loaded() {
            return;
        }
        // A movement packet was received this tick — tracked for SClientTickEnd zeroing.
        self.received_movement_this_tick
            .store(true, Ordering::Relaxed);
        if player.get_entity().has_vehicle() {
            return;
        }
        // Ignore movement packets while awaiting a teleport confirmation (vanilla behavior)
        if player
            .awaiting_teleport
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
        {
            return;
        }
        if player.is_movement_locked.load(Ordering::Relaxed) {
            self.force_tp(player, player.get_entity().pos.load());
            return;
        }
        // y = feet Y
        let position = packet.position;
        if position.x.is_nan() || position.y.is_nan() || position.z.is_nan() {
            self.try_kick(&TextComponent::translate_cross(
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                [],
            ));
            return;
        }
        let position = Vector3::new(
            Self::clamp_horizontal(position.x),
            Self::clamp_vertical(position.y),
            Self::clamp_horizontal(position.z),
        );
        if Self::sleeping_move(player, position, None) {
            return;
        }

        send_cancellable_blocking! {{
            server;
            PlayerMoveEvent {
                player: player.clone(),
                from: player.get_entity().pos.load(),
                to: position,
                cancelled: false,
            };

            'after: {
                let pos = event.to;
                let entity = &player.get_entity();
                let last_pos = entity.pos.load();

                let distance = last_pos.squared_distance_to_vec(&pos).sqrt();
                let cm = (distance * 100.0) as i32;
                if cm > 0 {
                    let stat = player.get_movement_statistic();
                    player.increment_stat(StatisticCategory::Custom, stat as i32, cm);
                }

                let height_difference = pos.y - last_pos.y;
                // Vanilla jumps before `player.move`, from where the player was.
                if entity.on_ground.load(Ordering::Relaxed) && packet.collision & FLAG_ON_GROUND == 0 && height_difference > 0.0 {
                    player.jump();
                }
                entity.set_pos(pos);

                let new_on_ground = packet.collision & FLAG_ON_GROUND != 0;
                entity.on_ground.store(new_on_ground, Ordering::Relaxed);
                Self::apply_player_move(
                    player,
                    last_pos,
                    pos,
                    new_on_ground,
                    packet.collision & FLAG_HORIZONTAL_COLLISION != 0,
                );
                if new_on_ground && entity.is_fall_flying() {
                    entity.set_fall_flying(false);
                }
                let world = &player.world();

                // TODO: Warn when player moves to quickly
                if !Self::sync_position(player, world, entity, pos, last_pos, packet.collision & FLAG_ON_GROUND != 0) {
                    // Send the new position to tracking players only.
                    world.send_to_tracking_players_editioned(
                        entity,
                        &CUpdateEntityPos::new(
                            player.entity_id().into(),
                            Vector3::new(
                                (encode_pos(pos.x) - encode_pos(last_pos.x)) as i16,
                                (encode_pos(pos.y) - encode_pos(last_pos.y)) as i16,
                                (encode_pos(pos.z) - encode_pos(last_pos.z)) as i16,
                            ),
                            packet.collision & FLAG_ON_GROUND != 0,
                        ),
                        &bedrock_move_player_packet(
                            entity,
                            pos,
                            CMovePlayer::MODE_NORMAL,
                            packet.collision & FLAG_ON_GROUND != 0,
                        ),
                    );
                }

                // Vanilla `doCheckFallDamage` with the client's move, then a move upwards ends
                // the fall.
                player.known_movement.store(pos.sub(&last_pos));
                entity.do_check_fall_damage(player.as_ref(), pos.sub(&last_pos), packet.collision & FLAG_ON_GROUND != 0);
                if height_difference > 0.0 {
                    entity.reset_fall_distance();
                }
                Self::try_reset_impulse_after_move(player, packet.collision & FLAG_ON_GROUND != 0);
                chunker::update_position(player);
                let delta = Vector3::new(
                    pos.x - last_pos.x,
                    pos.y - last_pos.y,
                    pos.z - last_pos.z,
                );
                // Only update idle timeout if there's actual movement (vanilla threshold)
                if delta.length_squared() > 1.0E-5 {
                    player.update_last_action_time();
                }
                player.progress_motion(delta);
            }

            'cancelled: {
                self.force_tp(player, player.get_entity().pos.load());
            }
        }}
    }

    #[expect(clippy::too_many_lines)]
    pub fn handle_position_rotation(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        packet: &SPlayerPositionRotation,
    ) {
        if !player.has_client_loaded() {
            return;
        }
        // A movement packet was received this tick — tracked for SClientTickEnd zeroing.
        self.received_movement_this_tick
            .store(true, Ordering::Relaxed);
        if player.get_entity().has_vehicle() {
            return;
        }
        // While a teleport waits for its confirmation, vanilla takes only the rotation
        // (`absSnapRotationTo`).
        if player
            .awaiting_teleport
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
        {
            if packet.yaw.is_finite() && packet.pitch.is_finite() {
                player
                    .get_entity()
                    .set_rotation(wrap_degrees(packet.yaw), wrap_degrees(packet.pitch));
            }
            return;
        }
        if player.is_movement_locked.load(Ordering::Relaxed) {
            let entity = player.get_entity();
            entity.set_rotation(packet.yaw, packet.pitch);
            self.force_tp(player, entity.pos.load());
            return;
        }
        // y = feet Y
        let position = packet.position;
        if !position.x.is_finite()
            || !position.y.is_finite()
            || !position.z.is_finite()
            || !packet.yaw.is_finite()
            || !packet.pitch.is_finite()
        {
            self.try_kick(&TextComponent::translate_cross(
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                [],
            ));
            return;
        }

        let position = Vector3::new(
            Self::clamp_horizontal(position.x),
            Self::clamp_vertical(position.y),
            Self::clamp_horizontal(position.z),
        );
        if Self::sleeping_move(
            player,
            position,
            Some((wrap_degrees(packet.yaw), wrap_degrees(packet.pitch))),
        ) {
            return;
        }

        send_cancellable_blocking! {{
            server;
            PlayerMoveEvent::new(
                player.clone(),
                player.get_entity().pos.load(),
                position,
            );

            'after: {
                let pos = event.to;
                let entity = &player.get_entity();
                let last_pos = entity.pos.load();

                let distance = last_pos.squared_distance_to_vec(&pos).sqrt();
                let cm = (distance * 100.0) as i32;
                if cm > 0 {
                    let stat = player.get_movement_statistic();
                    player.increment_stat(StatisticCategory::Custom, stat as i32, cm);
                }

                let height_difference = pos.y - last_pos.y;
                if entity.on_ground.load(Ordering::Relaxed)
                    && (packet.collision & FLAG_ON_GROUND) == 0
                    && height_difference > 0.0
                {
                    // Vanilla jumps before `player.move`, from where the player was.
                    player.jump();
                }
                entity.set_pos(pos);
                entity
                    .on_ground
                    .store((packet.collision & FLAG_ON_GROUND) != 0, Ordering::Relaxed);
                Self::apply_player_move(
                    player,
                    last_pos,
                    pos,
                    (packet.collision & FLAG_ON_GROUND) != 0,
                    (packet.collision & FLAG_HORIZONTAL_COLLISION) != 0,
                );

                entity.set_rotation(wrap_degrees(packet.yaw) % 360.0, wrap_degrees(packet.pitch));

                let entity_id = entity.entity_id;

                // TODO: use `pumpkin_util::math::pack_degrees`.
                let yaw = (entity.yaw.load() * 256.0 / 360.0).rem_euclid(256.0);
                let pitch = (entity.pitch.load() * 256.0 / 360.0).rem_euclid(256.0);
                let world = entity.world.load_full();

                // TODO: Warn when player moves to quickly
                if !Self::sync_position(
                    player,
                    &world,
                    entity,
                    pos,
                    last_pos,
                    (packet.collision & FLAG_ON_GROUND) != 0,
                ) {
                    // Send the new position to tracking players only.
                    world.send_to_tracking_players_editioned(
                        entity,
                        &CUpdateEntityPosRot::new(
                            entity_id.into(),
                            Vector3::new(
                                (encode_pos(pos.x) - encode_pos(last_pos.x)) as i16,
                                (encode_pos(pos.y) - encode_pos(last_pos.y)) as i16,
                                (encode_pos(pos.z) - encode_pos(last_pos.z)) as i16,
                            ),
                            yaw as u8,
                            pitch as u8,
                            (packet.collision & FLAG_ON_GROUND) != 0,
                        ),
                        &bedrock_move_player_packet(
                            entity,
                            pos,
                            CMovePlayer::MODE_NORMAL,
                            (packet.collision & FLAG_ON_GROUND) != 0,
                        ),
                    );
                }

                world.send_to_tracking_players(entity, &CHeadRot::new(entity_id.into(), yaw as u8));
                // Vanilla `doCheckFallDamage` with the client's move, then a move upwards ends
                // the fall.
                player.known_movement.store(pos.sub(&last_pos));
                entity.do_check_fall_damage(player.as_ref(), pos.sub(&last_pos), (packet.collision & FLAG_ON_GROUND) != 0);
                if height_difference > 0.0 {
                    entity.reset_fall_distance();
                }
                Self::try_reset_impulse_after_move(player, (packet.collision & FLAG_ON_GROUND) != 0);
                chunker::update_position(player);
                let delta = Vector3::new(
                    pos.x - last_pos.x,
                    pos.y - last_pos.y,
                    pos.z - last_pos.z,
                );
                // Only update idle timeout if there's actual movement (vanilla threshold)
                if delta.length_squared() > 1.0E-5 {
                    player.update_last_action_time();
                }
                player.progress_motion(delta);
            }

            'cancelled: {
                self.force_tp(player, position);
            }
        }}
    }

    /// The sleeping branch of vanilla `handlePlayerPositionChange`: a sleeper doesn't move or
    /// turn, and a move of more than a block is teleported back (with the packet's rotation).
    /// Measured from the current position (vanilla's `firstGood`, the position at the start
    /// of the player's tick). Returns whether the player sleeps.
    fn sleeping_move(
        player: &Player,
        position: Vector3<f64>,
        rotation: Option<(f32, f32)>,
    ) -> bool {
        if !player.is_sleeping() {
            return false;
        }
        let entity = player.get_entity();
        let current = entity.pos.load();
        if position.squared_distance_to_vec(&current) > 1.0 {
            // `Mth.wrapDegrees(packet.getYRot(player.getYRot()))`, likewise the pitch.
            let (yaw, pitch) = rotation.unwrap_or_else(|| {
                (
                    wrap_degrees(entity.yaw.load()),
                    wrap_degrees(entity.pitch.load()),
                )
            });
            player.request_teleport(current, yaw, pitch);
        }
        true
    }

    pub fn force_tp(&self, player: &Arc<Player>, position: Vector3<f64>) {
        let teleport_id = player.teleport_id_count.fetch_add(1, Ordering::Relaxed) + 1;
        *player
            .awaiting_teleport
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((teleport_id.into(), position));
        player.try_send_client_packet(&CPlayerPosition::new(
            teleport_id.into(),
            player.get_entity().pos.load(),
            Vector3::new(0.0, 0.0, 0.0),
            player.get_entity().yaw.load(),
            player.get_entity().pitch.load(),
            Vec::new(),
        ));
    }
}
