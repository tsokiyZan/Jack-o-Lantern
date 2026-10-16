use std::sync::{Arc, Mutex};

use crate::enchantment::condition::{EnchantmentLootContext, LootItemConditionExt};
use crate::enchantment::effects::EnchantmentEntityEffectExt;
use crate::enchantment::enchanted_item_in_use::EnchantedItemInUse;
use crate::enchantment::post_attack::do_post_attack;
use crate::entity::EntityBase;
use crate::entity::damage_source::DamageSource;
use crate::entity::living::LivingEntity;
use crate::entity::projectile::arrow::ArrowEntity;
use crate::world::World;
use pumpkin_data::data_component_impl::{EnchantmentsImpl, EquipmentSlot};
use pumpkin_data::enchantment::{
    ConditionalEffect, Enchantment, EnchantmentEntityEffect, EnchantmentTarget,
    EnchantmentValueEffect,
};
use pumpkin_data::item_stack::ItemStack;

/// Data-driven helper for enchantment effects matching vanilla `EnchantmentHelper`.
pub struct EnchantmentHelper;

impl EnchantmentHelper {
    /// Iterates through enchantments on an item stack, matching vanilla `runIterationOnItem`.
    pub fn run_iteration_on_item<F>(item_stack: &ItemStack, mut visitor: F)
    where
        F: FnMut(&'static Enchantment, i32),
    {
        if let Some(enchantments) = item_stack.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                visitor(enchantment, *level);
            }
        }
    }

    /// Applies projectile-spawned enchantment effects (e.g. Flame ignites the projectile for 100s).
    pub fn on_projectile_spawned(
        weapon: &ItemStack,
        projectile: &dyn EntityBase,
        arrow: Option<&ArrowEntity>,
    ) {
        let world = projectile.get_entity().world.load_full();
        let item = EnchantedItemInUse::ownerless(weapon);
        let position = projectile.get_entity().pos.load();
        Self::run_iteration_on_item(weapon, |enchantment, level| {
            // `Enchantment.onProjectileSpawned`, with `entityContext`.
            let context = EnchantmentLootContext::entity(&world, level, projectile, position);
            for conditional_effect in enchantment.effects.projectile_spawned {
                if !matches(conditional_effect, &context) {
                    continue;
                }
                conditional_effect
                    .effect
                    .apply(&world, level, &item, projectile, position);
                if let EnchantmentEntityEffect::Ignite { .. } = &conditional_effect.effect
                    && let Some(arrow) = arrow
                {
                    arrow.set_flame(true);
                }
            }
        });
    }

    /// Vanilla `runIterationOnItem(ItemStack, EquipmentSlot, LivingEntity, EnchantmentInSlotVisitor)`.
    fn run_iteration_on_item_in_slot<F>(
        item: &EnchantedItemInUse<'_>,
        slot: &EquipmentSlot,
        mut visitor: F,
    ) where
        F: FnMut(&'static Enchantment, i32, &EnchantedItemInUse<'_>),
    {
        if item.item_stack.is_empty() {
            return;
        }
        let Some(enchantments) = item.item_stack.get_data_component::<EnchantmentsImpl>() else {
            return;
        };
        for (enchantment, level) in enchantments.enchantment.iter() {
            if enchantment.matching_slot(slot) {
                visitor(enchantment, *level, item);
            }
        }
    }

    /// Vanilla `runIterationOnEquipment`.
    fn run_iteration_on_equipment<F>(owner: &dyn EntityBase, living: &LivingEntity, mut visitor: F)
    where
        F: FnMut(&'static Enchantment, i32, &EnchantedItemInUse<'_>),
    {
        for slot in &EquipmentSlot::VALUES {
            let stack = living.get_item_by_slot(owner, slot);
            let item = EnchantedItemInUse::in_slot(&stack, slot.clone(), owner);
            Self::run_iteration_on_item_in_slot(&item, slot, &mut visitor);
        }
    }

    /// Vanilla `EnchantmentHelper.doPostAttackEffects`: the victim's equipment, then the
    /// attacker's weapon.
    pub fn do_post_attack_effects(
        world: &Arc<World>,
        victim: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
    ) {
        let weapon = damage_source
            .causing
            .filter(|attacker| attacker.get_living_entity().is_some())
            .and_then(EntityBase::get_weapon_item);
        Self::do_post_attack_effects_with_item_source(
            world,
            victim,
            damage_source,
            weapon.as_ref(),
        );
    }

    /// Vanilla `EnchantmentHelper.doPostAttackEffectsWithItemSource`.
    pub fn do_post_attack_effects_with_item_source(
        world: &Arc<World>,
        victim: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
        source: Option<&ItemStack>,
    ) {
        Self::do_post_attack_effects_with_item_source_on_break(
            world,
            victim,
            damage_source,
            source.map(|stack| (stack, None)),
            None,
        );
    }

    /// Vanilla `EnchantmentHelper.doPostAttackEffectsWithItemSourceOnBreak`. `source` may name the
    /// stack it was read from when that stack lives outside the attacker's equipment, such as a
    /// thrown trident's item.
    pub fn do_post_attack_effects_with_item_source_on_break(
        world: &Arc<World>,
        victim: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
        source: Option<(&ItemStack, Option<&Mutex<ItemStack>>)>,
        attackerless_on_break: Option<&dyn Fn()>,
    ) {
        if let Some(living_victim) = victim.get_living_entity() {
            Self::run_iteration_on_equipment(victim, living_victim, |enchantment, level, item| {
                do_post_attack(
                    enchantment,
                    world,
                    level,
                    item,
                    EnchantmentTarget::Victim,
                    victim,
                    damage_source,
                );
            });
        }

        let Some((stack, detached_stack)) = source else {
            return;
        };
        if let Some(attacker) = damage_source.causing
            && attacker.get_living_entity().is_some()
        {
            let item = EnchantedItemInUse {
                detached_stack,
                ..EnchantedItemInUse::in_slot(stack, EquipmentSlot::MAIN_HAND, attacker)
            };
            Self::run_iteration_on_item_in_slot(
                &item,
                &EquipmentSlot::MAIN_HAND,
                |enchantment, level, item| {
                    do_post_attack(
                        enchantment,
                        world,
                        level,
                        item,
                        EnchantmentTarget::Attacker,
                        victim,
                        damage_source,
                    );
                },
            );
        } else if let Some(on_break) = attackerless_on_break {
            let item = EnchantedItemInUse {
                detached_stack,
                on_break: Some(on_break),
                ..EnchantedItemInUse::ownerless(stack)
            };
            Self::run_iteration_on_item(stack, |enchantment, level| {
                do_post_attack(
                    enchantment,
                    world,
                    level,
                    &item,
                    EnchantmentTarget::Attacker,
                    victim,
                    damage_source,
                );
            });
        }
    }

    /// Computes projectile spread angle using data-driven projectile spread effects.
    #[must_use]
    pub fn process_projectile_spread(weapon: &ItemStack, base_spread: f32) -> f32 {
        let mut spread = base_spread;
        if let Some(enchantments) = weapon.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_projectile_spread(*level, &mut spread);
            }
        }
        spread
    }

    /// Computes projectile count using data-driven projectile count effects.
    #[must_use]
    pub fn process_projectile_count(weapon: &ItemStack, base_count: usize) -> usize {
        let mut count = base_count as f32;
        if let Some(enchantments) = weapon.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_projectile_count(*level, &mut count);
            }
        }
        count as usize
    }

    /// Computes projectile piercing level using data-driven piercing effects.
    #[must_use]
    pub fn process_projectile_piercing(weapon: &ItemStack, base_piercing: u8) -> u8 {
        let mut piercing = f32::from(base_piercing);
        if let Some(enchantments) = weapon.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_piercing_count(*level, &mut piercing);
            }
        }
        piercing as u8
    }

    /// Vanilla `EnchantmentHelper.processAmmoUse`: the requirements see the ammo, not the weapon.
    #[must_use]
    pub fn process_ammo_use(
        world: &World,
        weapon: &ItemStack,
        ammo: &ItemStack,
        amount: i32,
    ) -> i32 {
        let mut value = amount as f32;
        Self::run_iteration_on_item(weapon, |enchantment, level| {
            let context = EnchantmentLootContext::item(world, level, ammo);
            apply_value_effects(enchantment.effects.ammo_use, &context, level, &mut value);
        });
        value as i32
    }

    /// Vanilla `EnchantmentHelper.modifyDamage`.
    #[must_use]
    pub fn modify_damage(
        world: &World,
        item: &ItemStack,
        victim: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
        damage: f32,
    ) -> f32 {
        Self::modify_damage_filtered_value(world, item, victim, damage_source, damage, |e| {
            e.effects.damage
        })
    }

    /// Vanilla `EnchantmentHelper.modifyFallBasedDamage`.
    #[must_use]
    pub fn modify_fall_based_damage(
        world: &World,
        item: &ItemStack,
        victim: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
        damage: f32,
    ) -> f32 {
        Self::modify_damage_filtered_value(world, item, victim, damage_source, damage, |e| {
            e.effects.smash_damage_per_fallen_block
        })
    }

    /// Vanilla `EnchantmentHelper.modifyKnockback`.
    #[must_use]
    pub fn modify_knockback(
        world: &World,
        item: &ItemStack,
        victim: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
        knockback: f32,
    ) -> f32 {
        Self::modify_damage_filtered_value(world, item, victim, damage_source, knockback, |e| {
            e.effects.knockback
        })
    }

    /// Vanilla `EnchantmentHelper.modifyArmorEffectiveness`.
    #[must_use]
    pub fn modify_armor_effectiveness(
        world: &World,
        item: &ItemStack,
        victim: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
        armor_fraction: f32,
    ) -> f32 {
        Self::modify_damage_filtered_value(
            world,
            item,
            victim,
            damage_source,
            armor_fraction,
            |e| e.effects.armor_effectiveness,
        )
    }

    /// Vanilla `Enchantment.modifyDamageFilteredValue` over every enchantment on `item`.
    fn modify_damage_filtered_value(
        world: &World,
        item: &ItemStack,
        victim: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
        value: f32,
        component: impl Fn(&Enchantment) -> &'static [ConditionalEffect<EnchantmentValueEffect>],
    ) -> f32 {
        let mut value = value;
        Self::run_iteration_on_item(item, |enchantment, level| {
            let context = EnchantmentLootContext::damage(world, level, victim, damage_source);
            apply_value_effects(component(enchantment), &context, level, &mut value);
        });
        value
    }

    /// Vanilla `EnchantmentHelper.processEquipmentDropChance`. Vanilla passes the killer as
    /// `entity`, so both passes read the killer's equipment.
    #[must_use]
    pub fn process_equipment_drop_chance(
        world: &World,
        entity: &dyn EntityBase,
        living: &LivingEntity,
        killing_blow: &DamageSource<'_>,
        chance: f32,
    ) -> f32 {
        let mut modified_chance = chance;
        let mut apply = |owner: &dyn EntityBase,
                         owner_living: &LivingEntity,
                         enchanted: EnchantmentTarget| {
            Self::run_iteration_on_equipment(owner, owner_living, |enchantment, level, _item| {
                let context = EnchantmentLootContext::damage(world, level, entity, killing_blow);
                for effect in enchantment.effects.equipment_drops {
                    if effect.enchanted == Some(enchanted)
                        && effect.affected == Some(EnchantmentTarget::Victim)
                        && effect
                            .requirements
                            .as_ref()
                            .is_none_or(|requirements| requirements.test(&context))
                    {
                        modified_chance = effect.effect.process(level, modified_chance);
                    }
                }
            });
        };
        apply(entity, living, EnchantmentTarget::Victim);
        if let Some(attacker) = killing_blow.causing
            && let Some(attacker_living) = attacker.get_living_entity()
        {
            apply(attacker, attacker_living, EnchantmentTarget::Attacker);
        }
        modified_chance
    }

    /// Vanilla `EnchantmentHelper.isImmuneToDamage` (Frost Walker against hot floors).
    #[must_use]
    pub fn is_immune_to_damage(
        world: &World,
        victim: &dyn EntityBase,
        living: &LivingEntity,
        damage_source: &DamageSource<'_>,
    ) -> bool {
        let mut immune = false;
        Self::run_iteration_on_equipment(victim, living, |enchantment, level, _item| {
            let context = EnchantmentLootContext::damage(world, level, victim, damage_source);
            immune = immune
                || enchantment
                    .effects
                    .damage_immunity
                    .iter()
                    .any(|effect| matches(effect, &context));
        });
        immune
    }

    /// Vanilla `EnchantmentHelper.getDamageProtection`, over all of the victim's equipment.
    #[must_use]
    pub fn get_damage_protection(
        world: &World,
        victim: &dyn EntityBase,
        living: &LivingEntity,
        damage_source: &DamageSource<'_>,
    ) -> f32 {
        let mut protection = 0.0;
        Self::run_iteration_on_equipment(victim, living, |enchantment, level, _item| {
            let context = EnchantmentLootContext::damage(world, level, victim, damage_source);
            apply_value_effects(
                enchantment.effects.damage_protection,
                &context,
                level,
                &mut protection,
            );
        });
        protection
    }

    /// Modifies crossbow charge time using data-driven charge time effects (e.g. Quick Charge).
    #[must_use]
    pub fn modify_crossbow_charge_time(weapon: &ItemStack, base_ticks: i32) -> i32 {
        let mut charge_time = base_ticks as f32;
        if let Some(enchantments) = weapon.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                let mut change_sec = 0.0f32;
                enchantment.modify_crossbow_charge_time(*level, &mut change_sec);
                charge_time += change_sec * 20.0;
            }
        }
        (charge_time as i32).max(0)
    }

    /// Vanilla `EnchantmentHelper.processDurabilityChange`.
    // TODO: `ItemStack::damage_item` in pumpkin-data still applies Unbreaking itself, with the
    // armour check its requirements make.
    #[must_use]
    pub fn process_durability_change(world: &World, item: &ItemStack, amount: i32) -> i32 {
        let mut value = amount as f32;
        Self::run_iteration_on_item(item, |enchantment, level| {
            let context = EnchantmentLootContext::item(world, level, item);
            apply_value_effects(enchantment.effects.item_damage, &context, level, &mut value);
        });
        value as i32
    }

    /// Modifies block experience using data-driven block experience effects (e.g. Fortune).
    #[must_use]
    pub fn modify_block_experience(tool: &ItemStack, base_xp: i32) -> i32 {
        let mut xp = base_xp as f32;
        if let Some(enchantments) = tool.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_block_experience(*level, &mut xp);
            }
        }
        (xp as i32).max(0)
    }

    /// Modifies mob experience using data-driven mob experience effects (e.g. Looting).
    #[must_use]
    pub fn modify_mob_experience(weapon: &ItemStack, base_xp: i32) -> i32 {
        let mut xp = base_xp as f32;
        if let Some(enchantments) = weapon.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_mob_experience(*level, &mut xp);
            }
        }
        (xp as i32).max(0)
    }

    /// Modifies durability to repair from experience using data-driven effects (e.g. Mending).
    #[must_use]
    pub fn modify_durability_to_repair_from_xp(item: &ItemStack, base_repair: f32) -> f32 {
        let mut repair = base_repair;
        if let Some(enchantments) = item.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_durability_to_repair_from_xp(*level, &mut repair);
            }
        }
        repair
    }

    /// Modifies trident return acceleration using data-driven effects (e.g. Loyalty).
    #[must_use]
    pub fn modify_trident_return_acceleration(trident: &ItemStack, base_accel: f32) -> f32 {
        let mut accel = base_accel;
        if let Some(enchantments) = trident.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_trident_return_to_owner_acceleration(*level, &mut accel);
            }
        }
        accel
    }

    /// Modifies trident spin attack strength using data-driven effects (e.g. Riptide).
    #[must_use]
    pub fn modify_trident_spin_attack_strength(trident: &ItemStack, base_strength: f32) -> f32 {
        let mut strength = base_strength;
        if let Some(enchantments) = trident.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_trident_spin_attack_strength(*level, &mut strength);
            }
        }
        strength
    }

    /// Modifies fishing time reduction using data-driven effects (e.g. Lure).
    #[must_use]
    pub fn modify_fishing_time_reduction(rod: &ItemStack, base_reduction: f32) -> f32 {
        let mut reduction = base_reduction;
        if let Some(enchantments) = rod.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_fishing_time_reduction(*level, &mut reduction);
            }
        }
        reduction
    }

    /// Modifies fishing luck bonus using data-driven effects (e.g. Luck of the Sea).
    #[must_use]
    pub fn modify_fishing_luck_bonus(rod: &ItemStack, base_luck: f32) -> f32 {
        let mut luck = base_luck;
        if let Some(enchantments) = rod.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_fishing_luck_bonus(*level, &mut luck);
            }
        }
        luck
    }

    /// Applies location-changed enchantment effects (e.g. Frost Walker replacing water with frosted ice).
    // TODO: vanilla `Enchantment.runLocationChangedEffects` tracks which location-based effects
    // are active per slot and deactivates them; Pumpkin runs them as plain entity effects and
    // always reports them inactive to `enchantment_active_check`.
    pub fn on_location_changed(
        entity: &dyn EntityBase,
        item: &ItemStack,
        position: pumpkin_util::math::vector3::Vector3<f64>,
    ) {
        let world = entity.get_entity().world.load_full();
        let item_in_use = EnchantedItemInUse::in_slot(item, EquipmentSlot::FEET, entity);
        Self::run_iteration_on_item(item, |enchantment, level| {
            let context = EnchantmentLootContext::location(&world, level, entity, false);
            for conditional_effect in enchantment.effects.location_changed {
                if matches(conditional_effect, &context) {
                    conditional_effect
                        .effect
                        .apply(&world, level, &item_in_use, entity, position);
                }
            }
        });
    }

    /// Vanilla `EnchantmentHelper.onHitBlock`.
    pub fn on_hit_block(
        weapon: &ItemStack,
        projectile: &dyn EntityBase,
        position: pumpkin_util::math::vector3::Vector3<f64>,
    ) {
        let world = projectile.get_entity().world.load_full();
        let item = EnchantedItemInUse::ownerless(weapon);
        Self::run_iteration_on_item(weapon, |enchantment, level| {
            let context = EnchantmentLootContext::entity(&world, level, projectile, position);
            for conditional_effect in enchantment.effects.hit_block {
                if matches(conditional_effect, &context) {
                    conditional_effect
                        .effect
                        .apply(&world, level, &item, projectile, position);
                }
            }
        });
    }
}

/// Vanilla `ConditionalEffect.matches`.
fn matches<T>(effect: &ConditionalEffect<T>, context: &EnchantmentLootContext<'_>) -> bool {
    effect
        .requirements
        .as_ref()
        .is_none_or(|requirements| requirements.test(context))
}

/// Vanilla `Enchantment.applyEffects` for value effects.
fn apply_value_effects(
    effects: &[ConditionalEffect<EnchantmentValueEffect>],
    context: &EnchantmentLootContext<'_>,
    level: i32,
    value: &mut f32,
) {
    for effect in effects {
        if matches(effect, context) {
            *value = effect.effect.process(level, *value);
        }
    }
}
