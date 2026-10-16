use std::sync::Arc;

use pumpkin_data::data_component_impl::{DamageImpl, EquipmentSlot};
use pumpkin_data::enchantment::LevelBasedValue;
use pumpkin_data::item_stack::DamageResult;
use pumpkin_util::math::vector3::Vector3;

use super::EnchantmentEntityEffectExt;
use crate::enchantment::EnchantedItemInUse;
use crate::entity::{EntityBase, equipment_break_status};
use crate::world::World;

/// Enchantment entity effect that alters item damage.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangeItemDamage {
    pub amount: LevelBasedValue,
}

impl ChangeItemDamage {
    #[must_use]
    pub const fn new(amount: LevelBasedValue) -> Self {
        Self { amount }
    }
}

impl EnchantmentEntityEffectExt for ChangeItemDamage {
    fn apply(
        &self,
        _world: &Arc<World>,
        enchantment_level: i32,
        item: &EnchantedItemInUse<'_>,
        _entity: &dyn EntityBase,
        _position: Vector3<f64>,
    ) {
        let stack = item.item_stack;
        if stack.get_max_damage().is_none() || stack.get_data_component::<DamageImpl>().is_none() {
            return;
        }
        let change = self.amount.calculate(enchantment_level) as i32;
        if let Some(detached) = item.detached_stack {
            // `hurtAndBreak` on the stack itself: no infinite materials for creative players.
            if item
                .owner
                .and_then(EntityBase::get_player)
                .is_some_and(crate::entity::player::Player::is_creative)
            {
                return;
            }
            let result = detached
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .damage_item(change);
            if result == DamageResult::Broken {
                if let Some(on_break) = item.on_break {
                    on_break();
                } else if let (Some(owner), Some(slot)) = (item.owner, &item.in_slot) {
                    on_equipped_item_broken(owner, slot);
                }
            }
        } else if let (Some(owner), Some(slot)) = (item.owner, &item.in_slot) {
            hurt_and_break_in_slot(owner, slot, change);
        }
    }
}

/// Vanilla `ItemStack.hurtAndBreak(int, LivingEntity, EquipmentSlot)` on the stack the owner wears
/// in `slot`.
fn hurt_and_break_in_slot(owner: &dyn EntityBase, slot: &EquipmentSlot, amount: i32) {
    if let Some(player) = owner.get_player() {
        player.damage_item_in_slot(slot, amount);
        return;
    }
    let Some(living) = owner.get_living_entity() else {
        return;
    };
    let (result, stack) = {
        let mut equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut stack = equipment.get(slot);
        let result = stack.damage_item(amount);
        if result != DamageResult::Untouched {
            equipment.put(slot, stack.clone());
        }
        (result, stack)
    };
    if result == DamageResult::Untouched {
        return;
    }
    living.send_equipment_changes(&[(slot.clone(), stack)]);
    if result == DamageResult::Broken {
        on_equipped_item_broken(owner, slot);
    }
}

/// Vanilla `LivingEntity.onEquippedItemBroken`.
fn on_equipped_item_broken(owner: &dyn EntityBase, slot: &EquipmentSlot) {
    let entity = owner.get_entity();
    entity
        .world
        .load()
        .send_entity_status(entity, equipment_break_status(slot), None);
}
