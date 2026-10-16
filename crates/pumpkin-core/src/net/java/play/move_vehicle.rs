#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_move_vehicle(&self, player: &Arc<Player>, packet: &SMoveVehicle) {
        // A movement packet was received this tick — tracked for SClientTickEnd zeroing.
        self.received_movement_this_tick
            .store(true, Ordering::Relaxed);
        let entity = player.get_entity();
        let last_pos = entity.pos.load();
        let pos = Vector3::new(packet.x, packet.y, packet.z);
        let vehicle = entity
            .vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(vehicle) = vehicle {
            let vehicle_entity = vehicle.get_entity();
            let old_pos = vehicle_entity.pos.load();
            if let Some(living) = vehicle.get_living_entity()
                && living.on_climbable(vehicle.as_ref())
            {
                vehicle_entity.reset_fall_distance();
            }
            vehicle_entity.set_pos(pos);
            vehicle_entity.set_rotation(packet.yaw, packet.pitch);
            // Vanilla `handleMoveVehicle`: the client's move lands the vehicle like its own.
            let movement = pos.sub(&old_pos);
            player.known_movement.store(movement);
            vehicle_entity
                .on_ground
                .store(packet.on_ground, Ordering::Relaxed);
            vehicle_entity.do_check_fall_damage(vehicle.as_ref(), movement, packet.on_ground);
        }
        entity.set_pos(pos);
        let distance = last_pos.squared_distance_to_vec(&pos).sqrt();
        let cm = (distance * 100.0).round() as i32;
        if cm > 0 {
            let stat = player.get_movement_statistic();
            player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Custom,
                stat as i32,
                cm,
            );
        }
        chunker::update_position(player);
    }
}
