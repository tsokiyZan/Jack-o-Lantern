use std::sync::{Arc, Mutex};

use crate::enchantment::condition::{EnchantmentLootContext, LootItemConditionExt};
use crate::enchantment::effects::EnchantmentEntityEffectExt;
use crate::enchantment::enchanted_item_in_use::EnchantedItemInUse;
use crate::enchantment::post_attack::do_post_attack;
use crate::entity::EntityBase;
use crate::entity::attributes::{Modifier, ModifierOperation, send_attribute_updates_for_living};
use crate::entity::damage_source::DamageSource;
use crate::entity::living::LivingEntity;
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::projectile::arrow::ArrowEntity;
use crate::world::World;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::{EnchantableImpl, EnchantmentsImpl, EquipmentSlot};
use pumpkin_data::enchantment::{
    ConditionalEffect, Enchantment, EnchantmentEntityEffect, EnchantmentTarget,
    EnchantmentValueEffect,
};
use pumpkin_data::enchantment_provider::{EnchantmentProvider, EnchantmentProviderKind};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::{DamageResult, ItemStack};
use pumpkin_util::math::java_round_f32;
use pumpkin_util::random::RandomImpl;
use pumpkin_util::random::weighted;

/// Data-driven helper for enchantment effects matching vanilla `EnchantmentHelper`.
pub struct EnchantmentHelper;

impl EnchantmentHelper {
    /// Vanilla `EnchantmentHelper.selectEnchantment`: the enchantments (as `EnchantmentInstance`
    /// pairs) the enchanting table, loot functions and mob equipment give a stack at `cost`.
    pub fn select_enchantment<R: RandomImpl>(
        random: &mut R,
        stack: &ItemStack,
        mut cost: i32,
        source: &[&'static Enchantment],
    ) -> Vec<(&'static Enchantment, i32)> {
        let mut results = Vec::new();
        let Some(enchantable) = stack.get_data_component::<EnchantableImpl>() else {
            return results;
        };

        cost += 1
            + random.next_bounded_i32(enchantable.value / 4 + 1)
            + random.next_bounded_i32(enchantable.value / 4 + 1);
        let random_span = (random.next_f32() + random.next_f32() - 1.0) * 0.15;
        cost = java_round_f32(cost as f32 + cost as f32 * random_span).max(1);

        let mut enchantments = Self::get_available_enchantment_results(cost, stack, source);
        if !enchantments.is_empty() {
            let weight = |instance: &(&'static Enchantment, i32)| instance.0.weight;
            if let Some(picked) = weighted::get_random_item(random, &enchantments, weight) {
                results.push(*picked);
            }

            while random.next_bounded_i32(50) <= cost {
                if let Some(last) = results.last() {
                    Self::filter_compatible_enchantments(&mut enchantments, last.0);
                }
                if enchantments.is_empty() {
                    break;
                }
                if let Some(picked) = weighted::get_random_item(random, &enchantments, weight) {
                    results.push(*picked);
                }
                cost /= 2;
            }
        }
        results
    }

    /// Vanilla `EnchantmentHelper.enchantItemFromProvider`: the provider's enchantments, each
    /// merged into `stack` with `ItemEnchantments.Mutable.upgrade`.
    pub fn enchant_item_from_provider<R: RandomImpl>(
        stack: &mut ItemStack,
        provider: &EnchantmentProvider,
        difficulty: &RegionalDifficulty,
        random: &mut R,
    ) {
        let enchantments = match provider.kind {
            // `SingleEnchantment.enchant`; the level is a constant, which samples without drawing.
            EnchantmentProviderKind::Single { enchantment, level } => {
                vec![(enchantment, level.clamp(1, enchantment.max_level))]
            }
            // `EnchantmentsByCostWithDifficulty.enchant`: `Mth.randomBetweenInclusive` always draws.
            EnchantmentProviderKind::ByCostWithDifficulty {
                enchantments,
                min_cost,
                max_cost_span,
            } => {
                let max_cost =
                    min_cost + (difficulty.special_multiplier * max_cost_span as f32) as i32;
                let cost = random.next_bounded_i32(max_cost - min_cost + 1) + min_cost;
                Self::select_enchantment(random, stack, cost, enchantments)
            }
            // `EnchantmentsByCost.enchant`, with a constant cost.
            EnchantmentProviderKind::ByCost { enchantments, cost } => {
                Self::select_enchantment(random, stack, cost, enchantments)
            }
        };
        for (enchantment, level) in enchantments {
            stack.enchant(enchantment, level);
        }
    }

    /// Vanilla `EnchantmentHelper.filterCompatibleEnchantments`.
    pub fn filter_compatible_enchantments(
        enchantments: &mut Vec<(&'static Enchantment, i32)>,
        target: &'static Enchantment,
    ) {
        enchantments.retain(|(enchantment, _)| target.are_compatible(enchantment));
    }

    /// Vanilla `EnchantmentHelper.getAvailableEnchantmentResults`, in `source` order.
    #[must_use]
    pub fn get_available_enchantment_results(
        value: i32,
        stack: &ItemStack,
        source: &[&'static Enchantment],
    ) -> Vec<(&'static Enchantment, i32)> {
        let is_book = stack.item == &Item::BOOK;
        source
            .iter()
            .filter(|enchantment| enchantment.is_primary_item(stack.item) || is_book)
            .filter_map(|enchantment| {
                (1..=enchantment.max_level)
                    .rev()
                    .find(|&level| {
                        value >= enchantment.min_cost.calculate(level)
                            && value <= enchantment.max_cost.calculate(level)
                    })
                    .map(|level| (*enchantment, level))
            })
            .collect()
    }

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

    /// Vanilla `EnchantmentHelper.getEnchantmentLevel(Holder<Enchantment>, LivingEntity)`: the
    /// best level among the items in the slots `enchantment` works from.
    pub fn get_enchantment_level(
        enchantment: &'static Enchantment,
        caller: &dyn EntityBase,
        living: &LivingEntity,
    ) -> i32 {
        EquipmentSlot::VALUES
            .iter()
            .filter(|slot| enchantment.matching_slot(slot))
            .map(|slot| living.get_item_by_slot(caller, slot))
            .filter(|stack| !stack.is_empty())
            .map(|stack| stack.get_enchantment_level(enchantment))
            .max()
            .unwrap_or(0)
    }

    /// Vanilla `EnchantmentHelper.forEachModifier(ItemStack, EquipmentSlot, BiConsumer)`: the
    /// `minecraft:attributes` modifiers of the enchantments active in `slot`.
    pub fn for_each_modifier<F>(stack: &ItemStack, slot: &EquipmentSlot, mut f: F)
    where
        F: FnMut(&'static Attributes, Modifier),
    {
        Self::run_iteration_on_item(stack, |enchantment, level| {
            if !enchantment.matching_slot(slot) {
                return;
            }
            for effect in enchantment.effects.attributes {
                f(
                    effect.attribute,
                    Modifier {
                        id: attribute_effect_id(effect.id, slot),
                        amount: f64::from(effect.amount.calculate(level)),
                        operation: ModifierOperation::from_operation(effect.operation),
                    },
                );
            }
        });
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

    /// Vanilla `EnchantmentHelper.runLocationChangedEffects` over the equipment.
    pub fn run_location_changed_effects(entity: &dyn EntityBase, living: &LivingEntity) {
        let world = entity.get_entity().world.load_full();
        Self::run_iteration_on_equipment(entity, living, |enchantment, level, item| {
            run_enchantment_location_changed_effects(
                &world,
                enchantment,
                level,
                item,
                entity,
                living,
            );
        });
    }

    /// Vanilla `EnchantmentHelper.runLocationChangedEffects(ServerLevel, ItemStack, LivingEntity,
    /// EquipmentSlot)`: the location-changed effects of `stack` worn in `slot`.
    pub fn run_location_changed_effects_in_slot(
        entity: &dyn EntityBase,
        living: &LivingEntity,
        stack: &ItemStack,
        slot: &EquipmentSlot,
    ) {
        let world = entity.get_entity().world.load_full();
        let item = EnchantedItemInUse::in_slot(stack, slot.clone(), entity);
        Self::run_iteration_on_item_in_slot(&item, slot, |enchantment, level, item| {
            run_enchantment_location_changed_effects(
                &world,
                enchantment,
                level,
                item,
                entity,
                living,
            );
        });
    }

    /// Vanilla `EnchantmentHelper.stopLocationBasedEffects(ItemStack, LivingEntity,
    /// EquipmentSlot)` with `Enchantment.stopLocationBasedEffects`: the effects `stack`'s
    /// enchantments have active in `slot` are deactivated.
    pub fn stop_location_based_effects_in_slot(
        stack: &ItemStack,
        living: &LivingEntity,
        slot: &EquipmentSlot,
    ) {
        if stack.is_empty() {
            return;
        }
        let Some(enchantments) = stack.get_data_component::<EnchantmentsImpl>() else {
            return;
        };
        for (enchantment, _) in enchantments.enchantment.iter() {
            if !enchantment.matching_slot(slot) {
                continue;
            }
            stop_active_location_effects(living, |(active_slot, active, _)| {
                active_slot == slot && std::ptr::eq(*active, *enchantment)
            });
        }
    }

    /// Vanilla `EnchantmentHelper.stopLocationBasedEffects(entity)`: every slot.
    pub fn stop_location_based_effects(living: &LivingEntity) {
        for slot in &EquipmentSlot::VALUES {
            stop_active_location_effects(living, |(active_slot, _, _)| active_slot == slot);
        }
    }

    /// Vanilla `EnchantmentHelper.tickEffects`: the `tick` effects of everything worn.
    pub fn tick_effects(entity: &dyn EntityBase, living: &LivingEntity) {
        let world = entity.get_entity().world.load_full();
        let position = entity.get_entity().pos.load();
        Self::run_iteration_on_equipment(entity, living, |enchantment, level, item| {
            let context = EnchantmentLootContext::entity(&world, level, entity, position);
            for conditional_effect in enchantment.effects.tick {
                if matches(conditional_effect, &context) {
                    conditional_effect
                        .effect
                        .apply(&world, level, item, entity, position);
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

/// Deactivates and forgets the active location effects `stop` picks.
fn stop_active_location_effects(
    living: &LivingEntity,
    stop: impl Fn(&(EquipmentSlot, &'static Enchantment, usize)) -> bool,
) {
    let stopped: Vec<_> = {
        let mut active = living
            .active_location_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (stopped, kept) = active.drain(..).partition(|entry| stop(entry));
        *active = kept;
        stopped
    };
    for (slot, enchantment, index) in stopped {
        if let Some(effect) = enchantment.effects.location_changed.get(index) {
            location_effect_deactivated(&effect.effect, living, &slot);
        }
    }
}

/// Vanilla `Enchantment.runLocationChangedEffects`: each location-changed effect is applied
/// while its requirements hold (they can ask whether it already was), and deactivated once they
/// stop holding.
fn run_enchantment_location_changed_effects(
    world: &Arc<World>,
    enchantment: &'static Enchantment,
    level: i32,
    item: &EnchantedItemInUse<'_>,
    entity: &dyn EntityBase,
    living: &LivingEntity,
) {
    let Some(slot) = item.in_slot.clone() else {
        return;
    };
    let position = entity.get_entity().pos.load();
    for (index, conditional_effect) in enchantment.effects.location_changed.iter().enumerate() {
        let key = (slot.clone(), enchantment, index);
        let was_active = living
            .active_location_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|active| active_matches(active, &key));
        let context = EnchantmentLootContext::location(world, level, entity, was_active);
        if matches(conditional_effect, &context) {
            if !was_active {
                living
                    .active_location_effects
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(key);
            }
            location_effect_changed_block(
                &conditional_effect.effect,
                world,
                level,
                item,
                entity,
                living,
                position,
                !was_active,
            );
        } else if remove_active(living, &key) {
            location_effect_deactivated(&conditional_effect.effect, living, &slot);
        }
    }
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

/// Vanilla `ItemStack.hurtAndBreak`: the wear goes through `processDurabilityChange` (Unbreaking)
/// before it is applied.
pub trait HurtAndBreak {
    fn hurt_and_break(&mut self, world: &World, amount: i32) -> DamageResult;
}

impl HurtAndBreak for ItemStack {
    fn hurt_and_break(&mut self, world: &World, amount: i32) -> DamageResult {
        if !self.is_damageable() || self.is_unbreakable() {
            return DamageResult::Untouched;
        }
        let amount = if amount > 0 {
            EnchantmentHelper::process_durability_change(world, self, amount)
        } else {
            amount
        };
        self.damage_item(amount)
    }
}

fn active_matches(
    active: &(EquipmentSlot, &'static Enchantment, usize),
    key: &(EquipmentSlot, &'static Enchantment, usize),
) -> bool {
    active.0 == key.0 && std::ptr::eq(active.1, key.1) && active.2 == key.2
}

/// Removes `key` from the active location effects; whether it was active.
fn remove_active(
    living: &LivingEntity,
    key: &(EquipmentSlot, &'static Enchantment, usize),
) -> bool {
    let mut active = living
        .active_location_effects
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = active.len();
    active.retain(|entry| !active_matches(entry, key));
    active.len() != before
}

/// Vanilla `EnchantmentAttributeEffect.idForSlot`: one modifier per slot.
fn attribute_effect_id(id: &str, slot: &EquipmentSlot) -> String {
    let slot_name = match slot {
        EquipmentSlot::MainHand(data)
        | EquipmentSlot::OffHand(data)
        | EquipmentSlot::Feet(data)
        | EquipmentSlot::Legs(data)
        | EquipmentSlot::Chest(data)
        | EquipmentSlot::Head(data)
        | EquipmentSlot::Body(data)
        | EquipmentSlot::Saddle(data) => &data.name,
    };
    format!("{id}/{slot_name}")
}

/// Vanilla `EnchantmentLocationBasedEffect.onChangedBlock`: an attribute effect adds its
/// modifier when it becomes active; an entity effect is applied on every block change.
#[allow(clippy::too_many_arguments)]
fn location_effect_changed_block(
    effect: &EnchantmentEntityEffect,
    world: &Arc<World>,
    level: i32,
    item: &EnchantedItemInUse<'_>,
    entity: &dyn EntityBase,
    living: &LivingEntity,
    position: pumpkin_util::math::vector3::Vector3<f64>,
    became_active: bool,
) {
    match effect {
        EnchantmentEntityEffect::Attribute {
            id,
            attribute,
            amount,
            operation,
        } => {
            if became_active && let Some(slot) = &item.in_slot {
                let modifier = Modifier {
                    id: attribute_effect_id(id, slot),
                    amount: f64::from(amount.calculate(level)),
                    operation: ModifierOperation::from_operation(*operation),
                };
                living.update_attribute(attribute, |instance| {
                    instance.add_or_replace_modifier(modifier);
                });
                send_attribute_updates_for_living(living, vec![(*attribute).clone()]);
            }
        }
        EnchantmentEntityEffect::AllOf(effects) => {
            for effect in *effects {
                location_effect_changed_block(
                    effect,
                    world,
                    level,
                    item,
                    entity,
                    living,
                    position,
                    became_active,
                );
            }
        }
        _ => effect.apply(world, level, item, entity, position),
    }
}

/// Vanilla `EnchantmentLocationBasedEffect.onDeactivated`: an attribute effect takes its
/// modifier back; entity effects have nothing to undo.
fn location_effect_deactivated(
    effect: &EnchantmentEntityEffect,
    living: &LivingEntity,
    slot: &EquipmentSlot,
) {
    match effect {
        EnchantmentEntityEffect::Attribute { id, attribute, .. } => {
            let modifier_id = attribute_effect_id(id, slot);
            living.update_attribute(attribute, |instance| instance.remove_modifier(&modifier_id));
            send_attribute_updates_for_living(living, vec![(*attribute).clone()]);
        }
        EnchantmentEntityEffect::AllOf(effects) => {
            for effect in *effects {
                location_effect_deactivated(effect, living, slot);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_util::random::trace::{Draw, TraceRandom};
    use std::borrow::Cow;

    /// The `#minecraft:in_enchanting_table` enchantments in vanilla's resolved tag order.
    fn enchanting_table_source() -> Vec<&'static Enchantment> {
        pumpkin_data::tag::Enchantment::MINECRAFT_IN_ENCHANTING_TABLE
            .1
            .iter()
            .filter_map(|&id| Enchantment::from_id(id as u8))
            .collect()
    }

    #[test]
    fn select_enchantment_draws_in_vanilla_order() {
        let stack = ItemStack::new(1, &Item::IRON_SWORD);
        let source = enchanting_table_source();
        let mut random = TraceRandom::from_seed(7);
        let picked = EnchantmentHelper::select_enchantment(&mut random, &stack, 13, &source);

        // Iron swords have enchantability 14, so both draws are nextInt(14 / 4 + 1).
        assert_eq!(
            random.draws[..4],
            [
                Draw::NextIntBounded(4),
                Draw::NextIntBounded(4),
                Draw::NextFloat,
                Draw::NextFloat
            ]
        );
        assert!(matches!(random.draws[4], Draw::NextIntBounded(total) if total > 1));
        assert_eq!(random.draws[5], Draw::NextIntBounded(50));
        assert!(!picked.is_empty());
        assert!(picked.iter().all(|(e, _)| e.is_primary_item(stack.item)));
    }

    fn difficulty(special_multiplier: f32) -> RegionalDifficulty {
        RegionalDifficulty {
            base_difficulty: pumpkin_util::difficulty::Difficulty::Hard,
            effective_difficulty: 2.0 + special_multiplier * 2.0,
            special_multiplier,
        }
    }

    #[test]
    fn single_provider_with_constant_level_draws_nothing() {
        let mut stack = ItemStack::new(1, &Item::DIAMOND_AXE);
        let mut random = TraceRandom::from_seed(7);
        EnchantmentHelper::enchant_item_from_provider(
            &mut stack,
            &EnchantmentProvider::ENDERMAN_LOOT_DROP,
            &difficulty(1.0),
            &mut random,
        );
        assert!(random.draws.is_empty());
        assert_eq!(stack.get_enchantment_level(&Enchantment::SILK_TOUCH), 1);
    }

    #[test]
    fn by_cost_with_difficulty_draws_its_cost_first() {
        let mut stack = ItemStack::new(1, &Item::IRON_SWORD);
        let mut random = TraceRandom::from_seed(7);
        // `randomBetweenInclusive(5, 5 + (int)(0.5 * 17))`, then `selectEnchantment`.
        EnchantmentHelper::enchant_item_from_provider(
            &mut stack,
            &EnchantmentProvider::MOB_SPAWN_EQUIPMENT,
            &difficulty(0.5),
            &mut random,
        );
        assert_eq!(
            random.draws[..3],
            [
                Draw::NextIntBounded(9),
                Draw::NextIntBounded(4),
                Draw::NextIntBounded(4)
            ]
        );
    }

    #[test]
    fn select_enchantment_without_enchantable_draws_nothing() {
        let stack = ItemStack::new(1, &Item::STICK);
        let source = enchanting_table_source();
        let mut random = TraceRandom::from_seed(7);
        assert!(EnchantmentHelper::select_enchantment(&mut random, &stack, 30, &source).is_empty());
        assert!(random.draws.is_empty());
    }

    #[test]
    fn available_results_keep_source_order_and_highest_level() {
        let stack = ItemStack::new(1, &Item::IRON_SWORD);
        let source = [
            &Enchantment::SMITE,
            &Enchantment::SHARPNESS,
            &Enchantment::PROTECTION,
        ];
        // Sharpness 5 costs 45..=65, level 4 costs 34..=54.
        let results = EnchantmentHelper::get_available_enchantment_results(45, &stack, &source);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.id, Enchantment::SMITE.id);
        assert_eq!(
            (results[1].0.id, results[1].1),
            (Enchantment::SHARPNESS.id, 5)
        );

        // A book takes everything in range, whatever its primary items.
        let book = ItemStack::new(1, &Item::BOOK);
        let results = EnchantmentHelper::get_available_enchantment_results(45, &book, &source);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn for_each_modifier_yields_efficiency_for_the_main_hand_only() {
        let mut stack = ItemStack::new(1, &Item::IRON_PICKAXE);
        stack.set_data_component(EnchantmentsImpl {
            enchantment: Cow::Owned(vec![(&Enchantment::EFFICIENCY, 5)]),
        });

        let mut found = Vec::new();
        EnchantmentHelper::for_each_modifier(&stack, &EquipmentSlot::MAIN_HAND, |attribute, m| {
            found.push((attribute.id, m));
        });
        assert_eq!(found.len(), 1);
        let (attribute, modifier) = &found[0];
        assert_eq!(*attribute, Attributes::MINING_EFFICIENCY.id);
        assert_eq!(modifier.id, "minecraft:enchantment.efficiency/mainhand");
        assert!((modifier.amount - 26.0).abs() < f64::EPSILON);
        assert!(matches!(modifier.operation, ModifierOperation::Add));

        let mut off_hand_count = 0;
        EnchantmentHelper::for_each_modifier(&stack, &EquipmentSlot::OFF_HAND, |_, _| {
            off_hand_count += 1;
        });
        assert_eq!(off_hand_count, 0);
    }
}
