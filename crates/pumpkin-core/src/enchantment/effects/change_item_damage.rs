use std::sync::Arc;

use crate::enchantment::HurtAndBreak;
use pumpkin_data::data_component_impl::DamageImpl;
use pumpkin_data::enchantment::LevelBasedValue;
use pumpkin_data::item_stack::DamageResult;
use pumpkin_util::math::vector3::Vector3;

use super::EnchantmentEntityEffectExt;
use crate::enchantment::EnchantedItemInUse;
use crate::entity::EntityBase;
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
        entity: &dyn EntityBase,
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
            let (result, broken) = {
                let mut detached = detached
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let broken = detached.clone();
                let result = detached.hurt_and_break(&entity.get_entity().world.load(), change);
                (result, broken)
            };
            if result == DamageResult::Broken {
                if let Some(on_break) = item.on_break {
                    on_break();
                } else if let (Some(living), Some(slot)) = (
                    item.owner.and_then(EntityBase::get_living_entity),
                    &item.in_slot,
                ) {
                    living.on_equipped_item_broken(&broken, slot);
                }
            }
        } else if let (Some(owner), Some(slot)) = (item.owner, &item.in_slot)
            && let Some(living) = owner.get_living_entity()
        {
            living.hurt_and_break_in_slot(owner, slot, change);
        }
    }
}
