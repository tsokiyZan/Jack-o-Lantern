#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_rotation(&self, player: &Player, rotation: &SPlayerRotation) {
        if !player.has_client_loaded() {
            return;
        }
        // A movement packet was received this tick — tracked for SClientTickEnd zeroing.
        self.received_movement_this_tick
            .store(true, Ordering::Relaxed);
        if !rotation.yaw.is_finite() || !rotation.pitch.is_finite() {
            self.try_kick(&TextComponent::translate_cross(
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                [],
            ));
            return;
        }
        // Vanilla `handleMovePlayer`: nothing moves while the End credits roll (`wonGame`).
        if player.won_game.load(Ordering::Relaxed) {
            return;
        }
        let entity = &player.get_entity();
        // Vanilla `handleMovePlayer` for a packet without a position: no move, but the client's
        // flags (`setOnGroundWithMovement`) and the fall's end on the ground. While a teleport
        // waits for its confirmation, only the rotation is taken (`absSnapRotationTo`).
        if player
            .awaiting_teleport
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none()
        {
            entity.on_ground.store(rotation.ground, Ordering::Relaxed);
            entity
                .horizontal_collision
                .store(rotation.horizontal_collision, Ordering::Relaxed);
            player.known_movement.store(Vector3::default());
            entity.do_check_fall_damage(player, Vector3::default(), rotation.ground);
        }
        entity.set_rotation(
            wrap_degrees(rotation.yaw) % 360.0,
            wrap_degrees(rotation.pitch),
        );
        // Send the new position to tracking players only.
        let entity_id = entity.entity_id;
        // TODO: use `pumpkin_util::math::pack_degrees`.
        let yaw = (entity.yaw.load() * 256.0 / 360.0).rem_euclid(256.0);
        let pitch = (entity.pitch.load() * 256.0 / 360.0).rem_euclid(256.0);

        let world = entity.world.load_full();
        let je_packet =
            CUpdateEntityRot::new(entity_id.into(), yaw as u8, pitch as u8, rotation.ground);

        let pos = entity.pos.load();

        // MODE_ROTATION not used for other players -> AvatarEntity always sends
        // MODE_NORMAL (client already lerps). MODE_ROTATION drops live head yaw on Bedrock.
        let be_packet =
            bedrock_move_player_packet(entity, pos, CMovePlayer::MODE_NORMAL, rotation.ground);

        world.send_to_tracking_players_editioned(entity, &je_packet, &be_packet);

        let je_packet = CHeadRot::new(entity_id.into(), yaw as u8);
        world.send_to_tracking_players(entity, &je_packet);
        Self::apply_move_speed_factor(player);
    }
}
