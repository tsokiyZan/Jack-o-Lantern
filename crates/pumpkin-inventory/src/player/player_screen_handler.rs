//! Player inventory screen handler.
//!
//! This module handles the player's inventory screen (opened with E key).
//! It includes:
//! - The 2x2 crafting grid (inventory crafting)
//! - Armor slots (head, chest, legs, feet)
//! - Main inventory (27 slots)
//! - Hotbar (9 slots)
//! - Offhand slot
//!
//! # Slot Layout
//!
//! The player screen handler uses the following slot indices:
//! - 0: Crafting result
//! - 1-4: Crafting grid (2x2)
//! - 5-8: Armor slots (head, chest, legs, feet)
//! - 9-35: Main inventory
//! - 36-44: Hotbar
//! - 45: Offhand

use super::player_inventory::PlayerInventory;
use crate::crafting::crafting_inventory::CraftingInventory;
use crate::crafting::crafting_screen_handler::CraftingScreenHandler;
use crate::crafting::recipes::{RecipeFinderScreenHandler, RecipeInputInventory};
use crate::inventory::Inventory;
use crate::screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour};
use crate::slot::{ArmorSlot, NormalSlot, Slot};
use pumpkin_data::data_component_impl::{EquipmentSlot, EquipmentType, EquippableImpl};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::screen::WindowType;
use std::any::Any;
use std::sync::Arc;

/// Screen handler for the player's inventory.
///
/// Manages the player's inventory UI including crafting, armor, and
/// the main inventory. This is the default screen shown when pressing E.
pub struct PlayerScreenHandler {
    /// Core screen handler behavior (slots, sync ID, listeners).
    behaviour: ScreenHandlerBehaviour,
    /// The 2x2 crafting grid inventory.
    crafting_inventory: Arc<dyn RecipeInputInventory>,
}

impl RecipeFinderScreenHandler for PlayerScreenHandler {}

impl CraftingScreenHandler<CraftingInventory> for PlayerScreenHandler {}

// TODO: Fully implement this
impl PlayerScreenHandler {
    /// Equipment slot order for armor display.
    const EQUIPMENT_SLOT_ORDER: [EquipmentSlot; 4] = [
        EquipmentSlot::HEAD,
        EquipmentSlot::CHEST,
        EquipmentSlot::LEGS,
        EquipmentSlot::FEET,
    ];

    /// Checks if a slot index is in the hotbar.
    ///
    /// Hotbar slots are 36-44 in the protocol (0-indexed 36-44).
    #[must_use]
    pub fn is_in_hotbar(slot: u8) -> bool {
        (36..=45).contains(&slot)
    }

    /// Gets a slot by its index.
    pub fn get_slot(&self, slot: usize) -> Arc<dyn Slot> {
        self.behaviour.slots[slot].clone()
    }

    /// Creates a new player screen handler.
    ///
    /// # Arguments
    /// - `player_inventory` - The player's inventory
    /// - `window_type` - The window type (usually None for player inventory)
    /// - `sync_id` - The synchronization ID
    pub fn new(
        player_inventory: &Arc<PlayerInventory>,
        window_type: Option<WindowType>,
        sync_id: u8,
        provider: Option<Arc<dyn crate::crafting::recipe_provider::RecipeProvider>>,
    ) -> Self {
        let crafting_inventory: Arc<dyn RecipeInputInventory> =
            Arc::new(CraftingInventory::new(2, 2));

        let mut player_screen_handler = Self {
            behaviour: ScreenHandlerBehaviour::new(sync_id, window_type),
            crafting_inventory: crafting_inventory.clone(),
        };

        player_screen_handler.add_recipe_slots(crafting_inventory, provider);

        // Add armor slots (head, chest, legs, feet)
        for i in 0..4 {
            player_screen_handler.add_slot(Arc::new(ArmorSlot::new(
                player_inventory.clone(),
                39 - i,
                Self::EQUIPMENT_SLOT_ORDER[i].clone(),
            )));
        }

        let player_inventory: Arc<dyn Inventory> = player_inventory.clone();

        // Add main inventory and hotbar
        player_screen_handler.add_player_slots(&player_inventory);

        // Offhand slot (index 40 in player inventory, 45 in screen handler)
        // TODO: onEquipStack callback for offhand
        player_screen_handler.add_slot(Arc::new(NormalSlot::new(player_inventory.clone(), 40)));

        player_screen_handler
    }
}

impl ScreenHandler for PlayerScreenHandler {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_behaviour(&self) -> &ScreenHandlerBehaviour {
        &self.behaviour
    }

    fn get_behaviour_mut(&mut self) -> &mut ScreenHandlerBehaviour {
        &mut self.behaviour
    }

    fn on_closed(&mut self, player: &dyn InventoryPlayer) {
        self.default_on_closed(player);
        //TODO: this.craftingResultInventory.clear();
        self.drop_inventory(player, self.crafting_inventory.clone());
    }

    /// Performs quick move (shift-click) for the given slot.
    ///
    /// The quick move logic depends on the source slot:
    /// - Crafting result (0) -> Player inventory (from end)
    /// - Crafting grid (1-4) -> Player inventory (from start)
    /// - Armor slots (5-8) -> Player inventory, unequips
    /// - Armor items -> Armor slots if empty
    /// - Offhand items -> Offhand slot if empty
    /// - Main inventory (9-35) -> Hotbar
    /// - Hotbar (36-44) -> Main inventory
    fn quick_move(&mut self, player: &dyn InventoryPlayer, slot_index: i32) -> ItemStack {
        let slot = self.get_behaviour().slots[slot_index as usize].clone();

        // TODO: Equippable component

        if slot.has_stack() {
            let mut slot_stack = slot.get_stack();
            let stack_prev = slot_stack.clone();

            let equipment_slot = slot_stack
                .get_data_component::<EquippableImpl>()
                .map_or(&EquipmentSlot::MAIN_HAND, |equippable| equippable.slot);

            // Quick move logic based on source slot
            let success = if slot_index == 0 {
                // From crafting result slot (0) -> Player Inventory (9-45, from end)
                self.insert_item(&mut slot_stack, 9, 45, true)
            } else if (1..5).contains(&slot_index) {
                // From craft ingredient slots (1-4) -> Player Inventory (9-45, from start)
                self.insert_item(&mut slot_stack, 9, 45, false)
            } else if (5..9).contains(&slot_index) {
                // From armour slots (5-8) -> Player Inventory (9-45, from start)
                let result = self.insert_item(&mut slot_stack, 9, 45, false);

                if result {
                    player.enqueue_equipment_change(equipment_slot, ItemStack::EMPTY);
                }
                result
            } else if equipment_slot.slot_type() == EquipmentType::HumanoidArmor
                && self
                    .get_slot((8 - equipment_slot.get_entity_slot_id()) as usize)
                    .get_cloned_stack()
                    .is_empty()
            {
                // Into empty armour slots (5-8)
                let index = 8 - equipment_slot.get_entity_slot_id();
                let result = self.insert_item(&mut slot_stack, index, index + 1, false);

                if result {
                    player.enqueue_equipment_change(equipment_slot, &stack_prev);
                }
                result
            } else if matches!(equipment_slot, EquipmentSlot::OffHand(_))
                && slot_index != 45
                && self.get_slot(45).get_cloned_stack().is_empty()
            {
                // Into empty offhand slot (45)
                let index = 45;
                self.insert_item(&mut slot_stack, index, index + 1, false)
            } else if (9..36).contains(&slot_index) {
                // From main inventory (9-35) -> Hotbar (36-44)
                self.insert_item(&mut slot_stack, 36, 45, false)
            } else if (36..45).contains(&slot_index) {
                // From hotbar (36-44) -> Main inventory (9-35)
                self.insert_item(&mut slot_stack, 9, 36, false)
            } else {
                // Fallback to moving into the player inventory area
                self.insert_item(&mut slot_stack, 9, 45, false)
            };

            if !success {
                return ItemStack::EMPTY.clone();
            }

            let stack = slot_stack.clone();

            if stack.is_empty() {
                slot.set_stack_prev(ItemStack::EMPTY.clone(), stack_prev.clone());
            } else {
                slot.set_stack(stack.clone());
            }

            if stack.item_count == stack_prev.item_count {
                return ItemStack::EMPTY.clone();
            }

            let mut taken_stack = stack_prev.clone();
            taken_stack.set_count(stack_prev.item_count - stack.item_count);
            slot.on_take_item(player, &taken_stack);

            if slot_index == 0 {
                // From crafting result slot (0)
                // Notify the result slot to refill
                slot.on_quick_move_crafted(stack.clone(), stack_prev.clone());
                // For crafting result slot, drop any remaining items
                if !stack.is_empty() {
                    player.drop_item(stack, false);
                }
            }

            return stack_prev;
        }

        // Nothing changed
        ItemStack::EMPTY.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_equipment::EntityEquipment;
    use pumpkin_data::item::Item;
    use pumpkin_protocol::java::client::play::{
        CSetContainerContent, CSetContainerProperty, CSetContainerSlot, CSetCursorItem,
        CSetPlayerInventory, CSetSelectedSlot,
    };
    use std::sync::Mutex;

    struct TestPlayer {
        inventory: Arc<PlayerInventory>,
        removed: bool,
        dropped: Mutex<Vec<ItemStack>>,
    }

    impl InventoryPlayer for TestPlayer {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn drop_item(&self, item: ItemStack, _thrown_from_hand: bool) {
            self.dropped.lock().unwrap().push(item);
        }
        fn is_removed_not_changing_dimension(&self) -> bool {
            self.removed
        }
        fn get_inventory(&self) -> Arc<PlayerInventory> {
            self.inventory.clone()
        }
        fn has_infinite_materials(&self) -> bool {
            false
        }
        fn is_creative(&self) -> bool {
            false
        }
        fn experience_level(&self) -> i32 {
            0
        }
        fn add_experience_levels(&self, _levels: i32) {}
        fn enchantment_seed(&self) -> i32 {
            0
        }
        fn set_enchantment_seed(&self, _seed: i32) {}
        fn enqueue_inventory_packet(
            &self,
            _packet: &CSetContainerContent,
            _window_type: Option<WindowType>,
        ) {
        }
        fn enqueue_slot_packet(
            &self,
            _packet: &CSetContainerSlot,
            _window_type: Option<WindowType>,
            _total_slots: usize,
        ) {
        }
        fn enqueue_cursor_packet(&self, _packet: &CSetCursorItem) {}
        fn enqueue_property_packet(&self, _packet: &CSetContainerProperty) {}
        fn enqueue_slot_set_packet(&self, _packet: &CSetPlayerInventory) {}
        fn enqueue_set_held_item_packet(&self, _packet: &CSetSelectedSlot) {}
        fn enqueue_equipment_change(&self, _slot: &EquipmentSlot, _stack: &ItemStack) {}
        fn award_experience(&self, _amount: i32) {}
        fn increment_stat(
            &self,
            _category: pumpkin_data::statistic::StatisticCategory,
            _stat_id: i32,
            _amount: i32,
        ) {
        }
        fn play_block_sound(&self, _sound: pumpkin_data::sound::Sound, _pitch: f32) {}
    }

    /// Vanilla `InventoryMenu.removed` through `dropOrPlaceInInventory`: the carried stack and
    /// the 2x2 grid go back into the inventory, except for a player removed from its level
    /// (killed at its 20th death tick), whose stacks drop at it.
    #[test]
    fn closing_drops_carried_and_grid_only_for_a_removed_player() {
        for removed in [false, true] {
            let inventory = Arc::new(PlayerInventory::new(
                Arc::new(std::sync::Mutex::new(EntityEquipment::new())),
                Arc::new(rustc_hash::FxHashMap::default()),
            ));
            let player = TestPlayer {
                inventory: inventory.clone(),
                removed,
                dropped: Mutex::new(Vec::new()),
            };
            let mut handler = PlayerScreenHandler::new(&inventory, None, 0, None);
            handler
                .crafting_inventory
                .set_stack(1, ItemStack::new(3, &Item::OAK_PLANKS));
            *handler.get_behaviour().cursor_stack.lock().unwrap() = ItemStack::new(5, &Item::STICK);
            handler.on_closed(&player);

            let dropped = player.dropped.lock().unwrap();
            let in_inventory = (0..inventory.size())
                .filter(|&i| !inventory.get_stack(i).is_empty())
                .count();
            if removed {
                assert_eq!(dropped.len(), 2, "carried stack and grid dropped");
                assert_eq!(in_inventory, 0);
            } else {
                assert!(dropped.is_empty());
                assert_eq!(
                    in_inventory, 2,
                    "carried stack and grid back in the inventory"
                );
            }
            assert!(
                handler
                    .get_behaviour()
                    .cursor_stack
                    .lock()
                    .unwrap()
                    .is_empty()
            );
            assert!(handler.crafting_inventory.get_stack(1).is_empty());
        }
    }
}
