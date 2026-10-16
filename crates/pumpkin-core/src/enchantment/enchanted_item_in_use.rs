use std::sync::Mutex;

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item_stack::ItemStack;

use crate::entity::EntityBase;

/// Vanilla `EnchantedItemInUse`: the enchanted stack an effect runs for, the slot it is used from
/// and the entity using it.
pub struct EnchantedItemInUse<'a> {
    pub item_stack: &'a ItemStack,
    pub in_slot: Option<EquipmentSlot>,
    pub owner: Option<&'a dyn EntityBase>,
    /// Set when the stack is not held in the owner's equipment (a projectile's weapon or pickup
    /// stack). Vanilla then damages that stack itself rather than the owner's slot.
    pub detached_stack: Option<&'a Mutex<ItemStack>>,
    /// Vanilla `onBreak` for an item that has no owner slot to break in.
    pub on_break: Option<&'a dyn Fn()>,
}

impl<'a> EnchantedItemInUse<'a> {
    /// Vanilla `EnchantedItemInUse(ItemStack, EquipmentSlot, LivingEntity)`.
    #[must_use]
    pub const fn in_slot(
        item_stack: &'a ItemStack,
        slot: EquipmentSlot,
        owner: &'a dyn EntityBase,
    ) -> Self {
        Self {
            item_stack,
            in_slot: Some(slot),
            owner: Some(owner),
            detached_stack: None,
            on_break: None,
        }
    }

    /// An item that is not tied to any entity, such as a weapon for effects run without one.
    #[must_use]
    pub const fn ownerless(item_stack: &'a ItemStack) -> Self {
        Self {
            item_stack,
            in_slot: None,
            owner: None,
            detached_stack: None,
            on_break: None,
        }
    }
}
