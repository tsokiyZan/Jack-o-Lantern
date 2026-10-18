use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::inventory::Inventory;
use pumpkin_data::item_stack::ItemStack;

use crate::player::player_inventory::PlayerInventory;
use crate::screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour};
use crate::slot::{NormalSlot, Slot};

/// Vanilla `AbstractMountInventoryMenu.stillValid`, which the server checks every tick.
pub type MountValidityCheck = Box<dyn Fn(&dyn InventoryPlayer) -> bool + Send + Sync>;

/// Vanilla `AbstractMountInventoryMenu` with the slots of `HorseInventoryMenu` and
/// `NautilusInventoryMenu`: the saddle, the body armor, then the mount's own inventory.
pub struct MountScreenHandler {
    behaviour: ScreenHandlerBehaviour,
    pub mount_inventory: Arc<dyn Inventory>,
    still_valid: MountValidityCheck,
}

impl MountScreenHandler {
    pub const SLOT_SADDLE: usize = 0;
    pub const SLOT_BODY_ARMOR: usize = 1;
    pub const SLOT_INVENTORY_START: usize = 2;
    pub const INVENTORY_ROWS: usize = 3;

    /// Vanilla `AbstractMountInventoryMenu.getInventorySize`.
    #[must_use]
    pub const fn get_inventory_size(inventory_columns: usize) -> usize {
        inventory_columns * Self::INVENTORY_ROWS
    }

    /// `saddle_inventory` and `armor_inventory` are the mount's single-slot equipment
    /// containers (vanilla `Mob.createEquipmentSlotContainer`), whose `is_valid_slot_for` is
    /// the mount's `isEquippableInSlot`.
    pub fn new(
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        mount_inventory: Arc<dyn Inventory>,
        saddle_inventory: Arc<dyn Inventory>,
        armor_inventory: Arc<dyn Inventory>,
        still_valid: MountValidityCheck,
    ) -> Self {
        let mut handler = Self {
            behaviour: ScreenHandlerBehaviour::new(sync_id, None),
            mount_inventory,
            still_valid,
        };

        handler.add_slot(Arc::new(MountEquipmentSlot::new(saddle_inventory)));
        handler.add_slot(Arc::new(MountEquipmentSlot::new(armor_inventory)));

        let mount_size = handler.mount_inventory.size();
        for i in 0..mount_size {
            handler.add_slot(Arc::new(NormalSlot::new(
                handler.mount_inventory.clone(),
                i,
            )));
        }

        let pi: Arc<dyn Inventory> = player_inventory.clone();
        handler.add_player_slots(&pi);

        handler
    }
}

/// Vanilla `ArmorSlot` over a mount's single-slot equipment container.
struct MountEquipmentSlot {
    inventory: Arc<dyn Inventory>,
    id: AtomicU8,
}

impl MountEquipmentSlot {
    fn new(inventory: Arc<dyn Inventory>) -> Self {
        Self {
            inventory,
            id: AtomicU8::new(0),
        }
    }
}

impl Slot for MountEquipmentSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.inventory.clone()
    }

    fn get_index(&self) -> usize {
        0
    }

    fn set_id(&self, id: usize) {
        self.id.store(id as u8, Ordering::Relaxed);
    }

    /// Vanilla `ArmorSlot.mayPlace`: the mount's `isEquippableInSlot`.
    fn can_insert(&self, stack: &ItemStack) -> bool {
        self.inventory.is_valid_slot_for(0, stack)
    }

    fn mark_dirty(&self) {
        self.inventory.mark_dirty();
    }

    fn get_max_item_count(&self) -> u8 {
        1
    }

    // TODO: vanilla `ArmorSlot.mayPickup` keeps an item with curse of binding on.
}

impl ScreenHandler for MountScreenHandler {
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
    }

    fn can_use(&self, player: &dyn InventoryPlayer) -> bool {
        (self.still_valid)(player)
    }

    fn quick_move(&mut self, _player: &dyn InventoryPlayer, slot_index: i32) -> ItemStack {
        let mut clicked = ItemStack::EMPTY.clone();
        let slot = self.get_behaviour().slots.get(slot_index as usize).cloned();

        if let Some(slot) = slot {
            if !slot.has_stack() {
                return clicked;
            }

            let mut stack = slot.get_stack();
            clicked = stack.clone();

            let mount_container_size = self.mount_inventory.size();
            let player_container_start = 2 + mount_container_size as i32;
            let total_slots = self.get_behaviour().slots.len() as i32;

            if slot_index < player_container_start {
                if !self.insert_item(&mut stack, player_container_start, total_slots, true) {
                    return ItemStack::EMPTY.clone();
                }
            } else if self.get_behaviour().slots[1].can_insert(&stack)
                && !self.get_behaviour().slots[1].has_stack()
            {
                if !self.insert_item(&mut stack, 1, 2, false) {
                    return ItemStack::EMPTY.clone();
                }
            } else if self.get_behaviour().slots[0].can_insert(&stack)
                && !self.get_behaviour().slots[0].has_stack()
            {
                if !self.insert_item(&mut stack, 0, 1, false) {
                    return ItemStack::EMPTY.clone();
                }
            } else if mount_container_size == 0
                || !self.insert_item(&mut stack, 2, player_container_start, false)
            {
                let player_container_end = player_container_start + 27;
                let player_hotbar_start = player_container_end;
                let player_hotbar_end = player_hotbar_start + 9;

                if (player_hotbar_start..player_hotbar_end).contains(&slot_index) {
                    if !self.insert_item(
                        &mut stack,
                        player_container_start,
                        player_container_end,
                        false,
                    ) {
                        return ItemStack::EMPTY.clone();
                    }
                } else if (player_container_start..player_container_end).contains(&slot_index) {
                    if !self.insert_item(&mut stack, player_hotbar_start, player_hotbar_end, false)
                    {
                        return ItemStack::EMPTY.clone();
                    }
                } else if !self.insert_item(
                    &mut stack,
                    player_hotbar_start,
                    player_container_end,
                    false,
                ) {
                    return ItemStack::EMPTY.clone();
                }

                return ItemStack::EMPTY.clone();
            }

            if stack.is_empty() {
                slot.set_stack(ItemStack::EMPTY.clone());
            } else {
                slot.set_stack(stack);
            }
        }

        clicked
    }
}
