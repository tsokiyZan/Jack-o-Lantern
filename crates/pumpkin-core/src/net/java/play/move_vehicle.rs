#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_protocol::java::client::play::CMoveVehicle;
use pumpkin_util::math::boundingbox::BoundingBox;

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

        let old_aabb = vehicle_entity.bounding_box.load();
        if let Some(living) = vehicle.get_living_entity()
            && living.on_climbable(vehicle.as_ref())
        {
            vehicle_entity.reset_fall_distance();
        }
        vehicle.move_entity(vehicle.as_ref(), target.sub(&steered.last_good));

        // The move must end within 0.25 blocks of the client's position (vanilla's
        // `yDist > -0.5 || yDist < 0.5` always holds, so the height never counts) and must not
        // run into anything the vehicle wasn't already touching; otherwise it goes back where
        // it was and the client is corrected, as at the world border.
        let moved_to = vehicle_entity.pos.load();
        let x_dist = target.x - moved_to.x;
        let z_dist = target.z - moved_to.z;
        let moved_wrongly = x_dist * x_dist + z_dist * z_dist;
        let fail = moved_wrongly > 0.0625;
        if fail {
            warn!(
                "{} (vehicle of {}) moved wrongly! {}",
                vehicle_entity.entity_type.resource_name,
                player.gameprofile.name,
                moved_wrongly.sqrt()
            );
        }
        let world = vehicle_entity.world.load();
        if fail && no_collision(&world, vehicle.as_ref(), old_aabb)
            || is_entity_colliding_with_anything_new(&world, vehicle.as_ref(), old_aabb, target)
        {
            // `absSnapTo(oldX, oldY, oldZ, targetYRot, targetXRot)`.
            vehicle_entity.snap_to(old, Some(yaw % 360.0), Some(pitch.clamp(-90.0, 90.0)));
            Self::send_vehicle_position(player, vehicle_entity);
            vehicle_entity.remove_latest_movement_recording();
            return;
        }

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

/// Vanilla `CollisionGetter.noCollision(entity, aabb)`: no block, entity or (near it) world
/// border collision in the box.
fn no_collision(world: &World, entity: &dyn EntityBase, aabb: BoundingBox) -> bool {
    if !world.get_block_collisions(aabb, entity).0.is_empty()
        || !world.get_entity_collisions(entity, aabb).is_empty()
    {
        return false;
    }
    let pos = entity.get_entity().pos.load();
    let border = world
        .worldborder
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    !border.is_inside_close_to_border(pos.x, pos.z, &aabb)
        || !border
            .collision_shapes()
            .iter()
            .any(|shape| shape.intersects(&aabb))
}

/// Vanilla `ServerGamePacketListenerImpl.isEntityCollidingWithAnythingNew`: at `target`, the
/// entity would overlap a block or entity it doesn't already overlap at `old_aabb`.
pub(super) fn is_entity_colliding_with_anything_new(
    world: &World,
    entity: &dyn EntityBase,
    old_aabb: BoundingBox,
    target: Vector3<f64>,
) -> bool {
    let deflate = f64::from(1.0E-5f32);
    let base = entity.get_entity();
    let new_aabb = base
        .bounding_box
        .load()
        .shift(target.sub(&base.pos.load()))
        .contract_all(deflate);
    let old_shape = old_aabb.contract_all(deflate);
    world
        .get_entity_collisions(entity, new_aabb)
        .into_iter()
        .chain(world.get_block_collisions(new_aabb, entity).0)
        .any(|shape| !shape.intersects(&old_shape))
}
