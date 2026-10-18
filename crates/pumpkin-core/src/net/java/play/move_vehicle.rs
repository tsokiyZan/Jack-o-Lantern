#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_protocol::java::client::play::CMoveVehicle;

impl JavaClient {
    /// Vanilla `ServerGamePacketListenerImpl.handleMoveVehicle`.
    #[expect(clippy::too_many_lines, reason = "mirrors vanilla handleMoveVehicle")]
    pub fn handle_move_vehicle(&self, player: &Arc<Player>, packet: &SMoveVehicle) {
        // A movement packet was received this tick — tracked for SClientTickEnd zeroing.
        self.received_movement_this_tick
            .store(true, Ordering::Relaxed);
        if !packet.x.is_finite()
            || !packet.y.is_finite()
            || !packet.z.is_finite()
            || !packet.yaw.is_finite()
            || !packet.pitch.is_finite()
        {
            self.try_kick(&TextComponent::translate_cross(
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_VEHICLE_MOVEMENT,
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_VEHICLE_MOVEMENT,
                [],
            ));
            return;
        }
        if !player.has_client_loaded()
            || player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some()
        {
            return;
        }
        let entity = player.get_entity();
        let Some(vehicle) = entity.root_vehicle() else {
            return;
        };
        let vehicle_entity = vehicle.get_entity();
        let target = Vector3::new(
            Self::clamp_horizontal(packet.x),
            Self::clamp_vertical(packet.y),
            Self::clamp_horizontal(packet.z),
        );
        let controlled = vehicle
            .controlling_passenger()
            .is_some_and(|controller| controller.get_entity().entity_id == player.entity_id());
        let mut steered = player
            .steered_vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !controlled {
            let tick_count = entity.tick_count.load(Ordering::Relaxed);
            if tick_count - steered.position_last_reset_at > 20
                || vehicle_entity.pos.load().squared_distance_to_vec(&target) > 1.0
            {
                warn!(
                    "{} was expected to be controlling vehicle {} but was not. Resetting vehicle position.",
                    player.gameprofile.name, vehicle_entity.entity_type.resource_name
                );
                Self::send_vehicle_position(player, vehicle_entity);
                steered.position_last_reset_at = tick_count;
            }
            return;
        }
        if steered.last_vehicle != Some(vehicle_entity.entity_id) {
            return;
        }

        let old = vehicle_entity.pos.load();
        let yaw = wrap_degrees(packet.yaw);
        let pitch = wrap_degrees(packet.pitch);
        let moved = target.sub(&steered.first_good).length_squared();
        let expected = vehicle_entity.velocity.load().length_squared();
        if moved - expected > 100.0 {
            let first_good = steered.first_good;
            warn!(
                "{} (vehicle of {}) moved too quickly! {},{},{}",
                vehicle_entity.entity_type.resource_name,
                player.gameprofile.name,
                target.x - first_good.x,
                target.y - first_good.y,
                target.z - first_good.z
            );
            Self::send_vehicle_position(player, vehicle_entity);
            return;
        }

        // `vehicle.move(MoverType.PLAYER, ...)`, for its step sounds and the block speed
        // factor. Vanilla then rejects a move that ends more than 0.25 blocks from the
        // client's or into a new collision; Pumpkin's `move` has no step height, so that
        // check would push a horse back off every block it steps up, and like a player's
        // own movement the vehicle's stays the client's.
        if let Some(living) = vehicle.get_living_entity()
            && living.on_climbable(vehicle.as_ref())
        {
            vehicle_entity.reset_fall_distance();
        }
        vehicle.move_entity(vehicle.as_ref(), target.sub(&steered.last_good));

        vehicle_entity.set_pos(target);
        vehicle_entity.set_rotation(yaw, pitch);
        let client_movement = target.sub(&old);
        // `handlePlayerKnownMovement`, `setOnGroundWithMovement` and `doCheckFallDamage`.
        player.known_movement.store(client_movement);
        vehicle_entity
            .on_ground
            .store(packet.on_ground, Ordering::Relaxed);
        vehicle_entity.check_supporting_block(
            vehicle.as_ref(),
            packet.on_ground,
            Some(client_movement),
        );
        vehicle_entity.do_check_fall_damage(vehicle.as_ref(), client_movement, packet.on_ground);
        steered.last_good = target;
        drop(steered);

        // The rider follows in its vehicle's tick (`positionRider`); the chunks follow the
        // player now, and the riding stats come from the vehicle's move
        // (`checkMovementStatistics`).
        chunker::update_position(player);
        let distance = client_movement.length();
        let cm = (distance * 100.0).round() as i32;
        if cm > 0 {
            let stat = player.get_movement_statistic();
            player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Custom,
                stat as i32,
                cm,
            );
        }
    }

    /// Vanilla `ClientboundMoveVehiclePacket.fromEntity`: puts the client's vehicle back where
    /// the server has it.
    fn send_vehicle_position(player: &Player, vehicle: &Entity) {
        let pos = vehicle.pos.load();
        player.try_send_client_packet(&CMoveVehicle::new(
            pos.x,
            pos.y,
            pos.z,
            vehicle.yaw.load(),
            vehicle.pitch.load(),
        ));
    }
}
