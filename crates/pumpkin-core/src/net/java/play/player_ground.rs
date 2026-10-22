#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_player_ground(&self, player: &Player, ground: &SSetPlayerGround) {
        // A movement packet was received this tick — tracked for SClientTickEnd zeroing.
        self.received_movement_this_tick
            .store(true, Ordering::Relaxed);
        // Vanilla `handleMovePlayer` for a packet without a position: no move, but the client's
        // flags (`setOnGroundWithMovement`) and the fall's end on the ground. Like a move, not
        // before the client has loaded or while a teleport waits for its confirmation.
        let entity = &player.living_entity.entity;
        if player.has_client_loaded()
            && !player.won_game.load(Ordering::Relaxed)
            && player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        {
            entity.on_ground.store(ground.on_ground, Ordering::Relaxed);
            entity
                .horizontal_collision
                .store(ground.horizontal_collision, Ordering::Relaxed);
            player.known_movement.store(Vector3::default());
            player.living_entity.entity.do_check_fall_damage(
                player,
                Vector3::default(),
                ground.on_ground,
            );
        }
        Self::apply_move_speed_factor(player);
    }
}
