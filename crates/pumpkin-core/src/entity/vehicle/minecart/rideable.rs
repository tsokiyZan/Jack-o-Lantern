use crate::item::interaction_result::InteractionResult;
use std::sync::Arc;

use crate::entity::{Entity, EntityBase, player::Player};

pub(super) struct RideableMinecart;

impl RideableMinecart {
    /// Vanilla `Minecart.interact`: `SUCCESS_SERVER` once the player rides, `CONSUME` when
    /// it couldn't.
    pub(super) fn interact(entity: &Entity, player: &Arc<Player>) -> InteractionResult {
        if player.get_entity().is_sneaking()
            || !entity
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        {
            return InteractionResult::PASS;
        }
        if player.get_entity().has_vehicle() {
            return InteractionResult::CONSUME;
        }

        let world = entity.world.load();
        let Some(vehicle) = world.get_entity_by_id(entity.entity_id) else {
            return InteractionResult::CONSUME;
        };
        let Some(passenger) = world.get_player_by_id(player.entity_id()) else {
            return InteractionResult::CONSUME;
        };

        entity.add_passenger(vehicle, passenger as Arc<dyn EntityBase>);
        InteractionResult::SUCCESS_SERVER
    }
}
