//! The inventory screens of vanilla `HasCustomInventoryScreen` mounts: horses, camels and the
//! nautilus.

use std::any::Any;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use arc_swap::ArcSwap;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::inventory::{
    Clearable, Inventory, SimpleInventory, sync_read_items_from_nbt,
};
use pumpkin_inventory::mount_screen_handler::MountScreenHandler;
use pumpkin_inventory::player::player_screen_handler::PlayerScreenHandler;
use pumpkin_inventory::screen_handler::{InventoryPlayer, ScreenHandler};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::java::client::play::COpenMountScreen;

use crate::entity::mob::Mob;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};

/// Vanilla `Mob.createEquipmentSlotContainer`: one equipment slot of a mount, as the mount
/// screen shows it.
pub struct EquipmentSlotContainer {
    mount: Arc<dyn EntityBase>,
    slot: EquipmentSlot,
}

impl EquipmentSlotContainer {
    fn mob(&self) -> Option<&dyn Mob> {
        self.mount.get_mob()
    }
}

impl Clearable for EquipmentSlotContainer {
    fn clear(&self) {
        self.set_stack(0, ItemStack::EMPTY.clone());
    }
}

impl Inventory for EquipmentSlotContainer {
    fn size(&self) -> usize {
        1
    }

    fn is_empty(&self) -> bool {
        self.get_stack(0).is_empty()
    }

    fn get_stack(&self, _slot: usize) -> ItemStack {
        self.mob().map_or_else(
            || ItemStack::EMPTY.clone(),
            |mob| mob.get_mob_entity().get_item_slot(&self.slot),
        )
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        let stack = self.get_stack(slot);
        self.set_stack(slot, ItemStack::EMPTY.clone());
        stack
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        let mut stack = self.get_stack(slot);
        let taken = stack.split(amount);
        self.set_stack(slot, stack);
        taken
    }

    /// Vanilla `setTheItem`: equips the item, which then always drops and keeps the mount.
    fn set_stack(&self, _slot: usize, stack: ItemStack) {
        let Some(mob) = self.mob() else {
            return;
        };
        let filled = !stack.is_empty();
        mob.set_item_slot_and_notify(&self.slot, stack);
        if filled {
            let mob_entity = mob.get_mob_entity();
            mob_entity.set_guaranteed_drop(&self.slot);
            mob_entity
                .persistence_required
                .store(true, Ordering::Relaxed);
        }
    }

    /// Vanilla `ArmorSlot.mayPlace`.
    fn is_valid_slot_for(&self, _slot: usize, stack: &ItemStack) -> bool {
        self.mob()
            .is_some_and(|mob| mob.is_equippable_in_slot(stack, &self.slot))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Vanilla `AbstractHorse.inventory` and `AbstractNautilus.inventory`, which `createInventory`
/// replaces when its size changes.
pub struct MountInventory(ArcSwap<SimpleInventory>);

impl Default for MountInventory {
    fn default() -> Self {
        Self(ArcSwap::from_pointee(SimpleInventory::new(0)))
    }
}

impl MountInventory {
    #[must_use]
    pub fn current(&self) -> Arc<SimpleInventory> {
        self.0.load_full()
    }

    /// Vanilla `createInventory`: a new container of `inventory_columns` columns that keeps
    /// what fits of the old one.
    pub fn create(&self, inventory_columns: usize) {
        let old = self.0.load_full();
        let new = SimpleInventory::new(MountScreenHandler::get_inventory_size(inventory_columns));
        for slot in 0..old.size().min(new.size()) {
            let stack = old.get_stack(slot);
            if !stack.is_empty() {
                new.set_stack(slot, stack);
            }
        }
        self.0.store(Arc::new(new));
    }

    /// Vanilla `hasInventoryChanged`.
    #[must_use]
    pub fn has_changed(&self, inventory: &Arc<SimpleInventory>) -> bool {
        !Arc::ptr_eq(&self.0.load(), inventory)
    }

    /// The `Items` of vanilla `AbstractChestedHorse.addAdditionalSaveData`.
    pub fn write_items(&self, nbt: &mut NbtCompound) {
        self.current().write_inventory_nbt(nbt, true);
    }

    /// The `Items` of vanilla `AbstractChestedHorse.readAdditionalSaveData`.
    pub fn read_items(&self, nbt: &NbtCompound) {
        let inventory = self.current();
        let mut stacks = vec![ItemStack::EMPTY.clone(); inventory.size()];
        sync_read_items_from_nbt(nbt, &mut stacks);
        for (slot, stack) in stacks.into_iter().enumerate() {
            inventory.set_stack(slot, stack);
        }
    }

    /// The inventory part of vanilla `AbstractHorse.dropEquipment`.
    pub fn drop_items(&self, mount: &Entity) {
        let inventory = self.current();
        for slot in 0..inventory.size() {
            let stack = inventory.get_stack(slot);
            // TODO: vanilla skips items enchanted with `prevent_equipment_drop`.
            if !stack.is_empty() {
                mount.spawn_at_location(stack);
            }
        }
    }
}

/// Vanilla `ServerPlayer.openHorseInventory` and `openNautilusInventory`. `is_current` is the
/// mount's `hasInventoryChanged`, negated.
pub fn open_mount_inventory(
    player: &Arc<Player>,
    mount: &dyn Mob,
    inventory: Arc<SimpleInventory>,
    inventory_columns: usize,
    is_current: impl Fn(&dyn Mob, &Arc<SimpleInventory>) -> bool + Send + Sync + 'static,
) {
    let entity = mount.get_entity();
    let Some(mount) = entity.world.load().get_entity_by_id(entity.entity_id) else {
        return;
    };
    let mount_id = entity.entity_id;
    let saddle: Arc<dyn Inventory> = Arc::new(EquipmentSlotContainer {
        mount: mount.clone(),
        slot: EquipmentSlot::SADDLE,
    });
    let armor: Arc<dyn Inventory> = Arc::new(EquipmentSlotContainer {
        mount: mount.clone(),
        slot: EquipmentSlot::BODY,
    });
    let mount_inventory: Arc<dyn Inventory> = inventory.clone();
    let weak_mount: Weak<dyn EntityBase> = Arc::downgrade(&mount);
    // Vanilla `AbstractMountInventoryMenu.stillValid` and the equipment containers'.
    let still_valid = Box::new(move |inventory_player: &dyn InventoryPlayer| {
        let Some(player) = inventory_player.as_any().downcast_ref::<Player>() else {
            return false;
        };
        let Some(mount) = weak_mount.upgrade() else {
            return false;
        };
        let Some(mob) = mount.get_mob() else {
            return false;
        };
        let entity = mount.get_entity();
        let range = player
            .living_entity
            .get_attribute_value(&Attributes::ENTITY_INTERACTION_RANGE)
            + 4.0;
        is_current(mob, &inventory)
            && entity.is_alive()
            && entity
                .bounding_box
                .load()
                .squared_magnitude(player.eye_position())
                < range * range
    });

    let Some(server) = player.world().server.upgrade() else {
        return;
    };
    let mut event = crate::plugin::api::events::inventory::inventory_open::InventoryOpenEvent::new(
        player.clone(),
    );
    server.plugin_manager.fire_blocking(&server, &mut event);
    if event.cancelled {
        return;
    }

    if !player
        .current_screen_handler
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_any()
        .is::<PlayerScreenHandler>()
    {
        player.close_handled_screen();
    }
    player.increment_screen_handler_sync_id();
    let sync_id = player.screen_handler_sync_id.load(Ordering::Relaxed);
    player.try_send_client_packet(&COpenMountScreen::new(
        sync_id,
        VarInt(inventory_columns as i32),
        mount_id,
    ));
    let handler: Arc<std::sync::Mutex<dyn ScreenHandler>> =
        Arc::new(std::sync::Mutex::new(MountScreenHandler::new(
            sync_id,
            &player.inventory,
            mount_inventory,
            saddle,
            armor,
            still_valid,
        )));
    player.on_screen_handler_opened(&handler);
    *player
        .current_screen_handler
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = handler;
    player.open_container_pos.store(None);
}
