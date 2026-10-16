use std::sync::Arc;

use pumpkin_data::enchantment::{
    Enchantment, EnchantmentEntityEffect, EnchantmentTarget, TargetedConditionalEffect,
};

use super::condition::{EnchantmentLootContext, LootItemConditionExt};
use super::effects::EnchantmentEntityEffectExt;
use super::enchanted_item_in_use::EnchantedItemInUse;
use crate::entity::EntityBase;
use crate::entity::damage_source::DamageSource;
use crate::world::World;

/// Vanilla `Enchantment.doPostAttack(ServerLevel, int, EnchantedItemInUse, EnchantmentTarget, ...)`.
pub fn do_post_attack(
    enchantment: &Enchantment,
    world: &Arc<World>,
    enchantment_level: i32,
    item: &EnchantedItemInUse<'_>,
    for_target: EnchantmentTarget,
    victim: &dyn EntityBase,
    damage_source: &DamageSource<'_>,
) {
    for effect in enchantment.effects.post_attack {
        if effect.enchanted == Some(for_target) {
            do_post_attack_effect(
                effect,
                world,
                enchantment_level,
                item,
                victim,
                damage_source,
            );
        }
    }
}

/// Vanilla static `Enchantment.doPostAttack(TargetedConditionalEffect, ...)`.
pub fn do_post_attack_effect(
    effect: &TargetedConditionalEffect<EnchantmentEntityEffect>,
    world: &Arc<World>,
    enchantment_level: i32,
    item: &EnchantedItemInUse<'_>,
    victim: &dyn EntityBase,
    damage_source: &DamageSource<'_>,
) {
    let context = EnchantmentLootContext::damage(world, enchantment_level, victim, damage_source);
    if effect
        .requirements
        .as_ref()
        .is_some_and(|requirements| !requirements.test(&context))
    {
        return;
    }
    let target = match effect.affected {
        Some(EnchantmentTarget::Attacker) => damage_source.causing,
        Some(EnchantmentTarget::DamagingEntity) => damage_source.direct,
        Some(EnchantmentTarget::Victim) => Some(victim),
        None => None,
    };
    if let Some(target) = target {
        effect.effect.apply(
            world,
            enchantment_level,
            item,
            target,
            target.get_entity().pos.load(),
        );
    }
}
