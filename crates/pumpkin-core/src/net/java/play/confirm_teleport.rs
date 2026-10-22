#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_confirm_teleport(&self, player: &Player, confirm_teleport: &SConfirmTeleport) {
        // Vanilla `handleAcceptTeleportPacket`: only the latest teleport's id counts; an older
        // one (the client acking a teleport the server already followed with another) is
        // ignored. The latest id acked with no position awaited disconnects.
        enum TeleportResult {
            Success,
            Stale,
            NotTeleporting,
        }

        let result = {
            let mut awaiting_teleport = player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((id, position)) = awaiting_teleport.as_ref() {
                if id == &confirm_teleport.teleport_id {
                    // We should set the position now to what we requested in the teleport packet.
                    // This may fix issues when the client sends the position while being teleported.
                    player.get_entity().set_pos(*position);
                    *awaiting_teleport = None;
                    TeleportResult::Success
                } else {
                    TeleportResult::Stale
                }
            } else if confirm_teleport.teleport_id.0
                == player
                    .teleport_id_count
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                TeleportResult::NotTeleporting
            } else {
                TeleportResult::Stale
            }
        };

        match result {
            TeleportResult::Success | TeleportResult::Stale => {}
            TeleportResult::NotTeleporting => {
                self.try_kick(&TextComponent::text(
                    "Send Teleport confirm, but we did not teleport",
                ));
            }
        }
    }
}
