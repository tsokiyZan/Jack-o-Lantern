use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;

use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;

pub struct CarrotOnAStickItem;
pub struct WarpedFungusOnAStickItem;

impl ItemMetadata for CarrotOnAStickItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::CARROT_ON_A_STICK.id])
    }
}

impl ItemBehaviour for CarrotOnAStickItem {
    /// Vanilla `FoodOnAStickItem.use`.
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let vehicle_opt = player
            .get_entity()
            .vehicle
            .try_lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let Some(vehicle) = vehicle_opt
            && vehicle.get_entity().entity_type.id == EntityType::PIG.id
            && let Some(steerable) = vehicle.get_item_steerable()
            && steerable.boost()
        {
            stick_boosted(player, hand, 7)
        } else {
            // Vanilla awards the stat only for a use that doesn't boost.
            player.award_item_used(item);
            InteractionResult::PASS
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ItemMetadata for WarpedFungusOnAStickItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::WARPED_FUNGUS_ON_A_STICK.id])
    }
}

impl ItemBehaviour for WarpedFungusOnAStickItem {
    /// Vanilla `FoodOnAStickItem.use`.
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let vehicle_opt = player
            .get_entity()
            .vehicle
            .try_lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let Some(vehicle) = vehicle_opt
            && vehicle.get_entity().entity_type.id == EntityType::STRIDER.id
            && let Some(steerable) = vehicle.get_item_steerable()
            && steerable.boost()
        {
            stick_boosted(player, hand, 1)
        } else {
            // Vanilla awards the stat only for a use that doesn't boost.
            player.award_item_used(item);
            InteractionResult::PASS
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The boost branch of vanilla `FoodOnAStickItem.use`: `hurtAndConvertOnBreak` into a fishing
/// rod, and the server's own swing.
fn stick_boosted(player: &Player, hand: Hand, damage: i32) -> InteractionResult {
    let before = player.inventory.get_stack_in_hand(hand);
    player.damage_item_in_hand(hand, damage);
    if !before.is_empty() && player.inventory.get_stack_in_hand(hand).is_empty() {
        player
            .inventory
            .set_stack_in_hand(hand, ItemStack::new(1, &Item::FISHING_ROD));
    }
    InteractionResult::SUCCESS_SERVER
}
