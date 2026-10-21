//! Mob equipment: vanilla `DifficultyInstance` for spawn equipment, the spawn enchantment
//! providers, and `Mob`'s equipment rules (`equipItemIfPossible`, `canReplaceCurrentItem`,
//! `pickUpItem`).

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::{
    AttributeModifiersImpl, CustomNameImpl, EnchantmentsImpl, EquipmentSlot, EquipmentType,
    EquippableImpl, IDSet, IdOr, Operation,
};
use pumpkin_data::enchantment_provider::EnchantmentProvider;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{Tag, Taggable};
use pumpkin_util::difficulty::Difficulty;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::mob::{Mob, MobEntity};

/// Vanilla `DropChances.DEFAULT_EQUIPMENT_DROP_CHANCE`.
pub const DEFAULT_EQUIPMENT_DROP_CHANCE: f32 = 0.085;

// ══════════════════════════════════════════════════════════════════
// Regional Difficulty — exact vanilla DifficultyInstance.java
// Vanilla approximation: identical formula to vanilla Minecraft 26.2's
// DifficultyInstance, including clamped regional difficulty, special
// multiplier (0-1 linear), and effective difficulty (2-4 range).
// ══════════════════════════════════════════════════════════════════

/// Computed difficulty values for a specific world chunk.
///
/// Mirrors Vanilla's `DifficultyInstance`. Used to scale equipment spawn rates,
/// enchantment costs, and loot-pickup flags.
#[derive(Clone, Copy)]
pub struct RegionalDifficulty {
    /// The world's base difficulty level (`Easy`, `Normal`, `Hard`).
    pub base_difficulty: Difficulty,
    /// Effective difficulty computed from game time, inhabited time, and moon phase.
    /// Clamped to the range `[2.0, 4.0]` (or `0.0` for Peaceful).
    pub effective_difficulty: f32,
    /// Linear multiplier in `[0.0, 1.0]` derived from `effective_difficulty`.
    /// When `0.0` (fresh chunk + early game), no equipment, enchantments, or
    /// loot-pickup flags are applied.
    pub special_multiplier: f32,
}

impl RegionalDifficulty {
    /// Computes difficulty at the given world position.
    ///
    /// Looks up the chunk's inhabited time and combines it with the world's
    /// difficulty, game time, and moon phase.
    pub fn at(world: &Arc<crate::world::World>, pos: Vector3<f64>) -> Self {
        let level_info = world.level_info.load();
        let difficulty = level_info.difficulty;
        let time_of_day = world.level_time.try_lock().map_or(0, |t| t.time_of_day);
        let inhabited_time = {
            let chunk_x = (pos.x / 16.0).floor() as i32;
            let chunk_z = (pos.z / 16.0).floor() as i32;
            world
                .level
                .loaded_chunks
                .get(&pumpkin_util::math::vector2::Vector2::new(chunk_x, chunk_z))
                .map_or(0, |c| {
                    c.inhabited_time.load(std::sync::atomic::Ordering::Relaxed)
                })
        };
        let moon_brightness = moon_brightness(time_of_day);

        Self::calculate(difficulty, time_of_day, inhabited_time, moon_brightness)
    }

    /// Direct calculation from raw inputs. Used by `at()` and for testing.
    #[must_use]
    pub fn calculate(
        difficulty: Difficulty,
        total_game_time: i64,
        chunk_inhabited_time: u64,
        moon_brightness: f32,
    ) -> Self {
        if difficulty == Difficulty::Peaceful {
            return Self {
                base_difficulty: difficulty,
                effective_difficulty: 0.0,
                special_multiplier: 0.0,
            };
        }

        let is_hard = difficulty == Difficulty::Hard;

        let mut scale = 0.75f32;
        let global_scale = ((total_game_time as f32 - 72000.0) / 1440000.0).clamp(0.0, 1.0) * 0.25;
        scale += global_scale;

        let mut local_scale = 0.0f32;
        local_scale += (chunk_inhabited_time as f32 / 3600000.0).clamp(0.0, 1.0)
            * if is_hard { 1.0 } else { 0.75 };
        local_scale += (moon_brightness * 0.25).clamp(0.0, global_scale);

        if difficulty == Difficulty::Easy {
            local_scale *= 0.5;
        }

        let difficulty_id = match difficulty {
            Difficulty::Peaceful => 0,
            Difficulty::Easy => 1,
            Difficulty::Normal => 2,
            Difficulty::Hard => 3,
        };

        let effective = difficulty_id as f32 * (scale + local_scale);

        let special_multiplier = if effective < 2.0 {
            0.0
        } else if effective > 4.0 {
            1.0
        } else {
            (effective - 2.0) / 2.0
        };

        Self {
            base_difficulty: difficulty,
            effective_difficulty: effective,
            special_multiplier,
        }
    }
}

/// Moon brightness factor for the given time of day (0.0 to 1.0).
/// Full moon at phase 0, new moon at phase 4.
#[must_use]
fn moon_brightness(time_of_day: i64) -> f32 {
    let phase = (time_of_day / 24000 % 8) as i32;
    (phase - 4).abs() as f32 / 4.0
}

// ══════════════════════════════════════════════════════════════════
// Enchantment system — powered by data-driven EnchantmentProvider
// ══════════════════════════════════════════════════════════════════

/// Applies mob spawn equipment enchantments to a stack using the data-driven
/// `minecraft:mob_spawn_equipment` enchantment provider.
pub fn apply_vanilla_enchantments(
    stack: &mut ItemStack,
    _slot: &EquipmentSlot,
    special_multiplier: f32,
) {
    EnchantmentProvider::MOB_SPAWN_EQUIPMENT.apply_to_stack(stack, special_multiplier);
}

/// Applies pillager crossbow enchantments based on raid wave or default spawn provider.
pub fn apply_pillager_crossbow_enchantments(stack: &mut ItemStack, wave: Option<u32>) {
    match wave {
        Some(w) if w >= 5 => {
            EnchantmentProvider::RAID_PILLAGER_POST_WAVE_5.apply_to_stack(stack, 0.0);
        }
        Some(w) if w >= 3 => {
            EnchantmentProvider::RAID_PILLAGER_POST_WAVE_3.apply_to_stack(stack, 0.0);
        }
        _ => {
            EnchantmentProvider::PILLAGER_SPAWN_CROSSBOW.apply_to_stack(stack, 0.0);
        }
    }
}

/// Applies vindicator weapon enchantments based on raid wave.
pub fn apply_vindicator_weapon_enchantments(stack: &mut ItemStack, wave: Option<u32>) {
    match wave {
        Some(w) if w >= 5 => {
            EnchantmentProvider::RAID_VINDICATOR_POST_WAVE_5.apply_to_stack(stack, 0.0);
        }
        Some(_) => {
            EnchantmentProvider::RAID_VINDICATOR.apply_to_stack(stack, 0.0);
        }
        None => {}
    }
}

#[must_use]
pub fn is_equippable_in_slot(
    entity_type: &EntityType,
    stack: &ItemStack,
    slot: &EquipmentSlot,
) -> bool {
    stack.get_data_component::<EquippableImpl>().map_or_else(
        || *slot == EquipmentSlot::MAIN_HAND,
        |equippable| {
            *equippable.slot == *slot
                && equippable
                    .allowed_entities
                    .as_ref()
                    .is_none_or(|allowed| match allowed {
                        IDSet::Tag(tag) => entity_type.is_tagged_with(tag).unwrap_or(false),
                        IDSet::IDs(ids) => ids.iter().any(|id| id.id == entity_type.id),
                    })
        },
    )
}

/// Vanilla `LivingEntity.getEquipmentSlotForItem`: the item's equippable slot when the mob
/// can use it, else the main hand.
#[must_use]
pub fn get_equipment_slot_for_item<M: Mob + ?Sized>(mob: &M, stack: &ItemStack) -> EquipmentSlot {
    stack
        .get_data_component::<EquippableImpl>()
        .filter(|equippable| mob.can_use_slot(equippable.slot))
        .map_or(EquipmentSlot::MAIN_HAND, |equippable| {
            equippable.slot.clone()
        })
}

#[must_use]
/// Whether `new_item` is an upgrade over `current_item` in `slot`.
pub fn can_replace_current_item(
    mob: &MobEntity,
    preferred_weapon_type: Option<&'static Tag>,
    new_item: &ItemStack,
    current_item: &ItemStack,
    slot: &EquipmentSlot,
) -> bool {
    if current_item.is_empty() {
        return true;
    }
    if slot.is_armor_slot() {
        compare_armor(mob, new_item, current_item, slot)
    } else {
        *slot == EquipmentSlot::MAIN_HAND
            && compare_weapons(mob, preferred_weapon_type, new_item, current_item, slot)
    }
}

fn compare_armor(
    mob: &MobEntity,
    new_item: &ItemStack,
    current_item: &ItemStack,
    slot: &EquipmentSlot,
) -> bool {
    if super::has_prevent_armor_change(current_item) {
        return false;
    }
    let new_defense = approximate_attribute_with(mob, new_item, &Attributes::ARMOR, slot);
    let old_defense = approximate_attribute_with(mob, current_item, &Attributes::ARMOR, slot);
    let new_toughness =
        approximate_attribute_with(mob, new_item, &Attributes::ARMOR_TOUGHNESS, slot);
    let old_toughness =
        approximate_attribute_with(mob, current_item, &Attributes::ARMOR_TOUGHNESS, slot);
    if new_defense != old_defense {
        return new_defense > old_defense;
    }
    if new_toughness != old_toughness {
        return new_toughness > old_toughness;
    }
    can_replace_equal_item(new_item, current_item)
}

fn compare_weapons(
    mob: &MobEntity,
    preferred_weapon_type: Option<&'static Tag>,
    new_item: &ItemStack,
    current_item: &ItemStack,
    slot: &EquipmentSlot,
) -> bool {
    if let Some(preferred) = preferred_weapon_type {
        let current_preferred = current_item.item.has_tag(preferred);
        let new_preferred = new_item.item.has_tag(preferred);
        if current_preferred && !new_preferred {
            return false;
        }
        if !current_preferred && new_preferred {
            return true;
        }
    }
    let new_damage = approximate_attribute_with(mob, new_item, &Attributes::ATTACK_DAMAGE, slot);
    let old_damage =
        approximate_attribute_with(mob, current_item, &Attributes::ATTACK_DAMAGE, slot);
    if new_damage != old_damage {
        return new_damage > old_damage;
    }
    can_replace_equal_item(new_item, current_item)
}

/// Vanilla `Mob.getApproximateAttributeWith`: the mob's base value (0 without the attribute)
/// through the item's modifiers for `slot`.
fn approximate_attribute_with(
    mob: &MobEntity,
    stack: &ItemStack,
    attribute: &Attributes,
    slot: &EquipmentSlot,
) -> f64 {
    let living = &mob.living_entity;
    let base_value = if living.has_attribute(attribute) {
        living.get_attribute_base(attribute)
    } else {
        0.0
    };
    stack
        .get_data_component::<AttributeModifiersImpl>()
        .map_or(base_value, |modifiers| {
            compute_attribute_modifiers(modifiers, attribute, base_value, slot)
        })
}

/// Vanilla `ItemAttributeModifiers.compute`: each matching modifier in order, the
/// multiplied-total ones scaling what the earlier ones made.
fn compute_attribute_modifiers(
    modifiers: &AttributeModifiersImpl,
    attribute: &Attributes,
    base_value: f64,
    slot: &EquipmentSlot,
) -> f64 {
    let mut value = base_value;
    for modifier in modifiers.attribute_modifiers.iter() {
        if modifier.slot.test(slot) && modifier.r#type.id == attribute.id {
            value += match modifier.operation {
                Operation::AddValue => modifier.amount,
                Operation::AddMultipliedBase => modifier.amount * base_value,
                Operation::AddMultipliedTotal => modifier.amount * value,
            };
        }
    }
    value
}

#[must_use]
pub fn can_replace_equal_item(new_item: &ItemStack, current_item: &ItemStack) -> bool {
    let enchantment_count = |stack: &ItemStack| {
        stack
            .get_data_component::<EnchantmentsImpl>()
            .map_or(0, |enchantments| enchantments.enchantment.len())
    };
    let new_enchantments = enchantment_count(new_item);
    let current_enchantments = enchantment_count(current_item);
    if new_enchantments != current_enchantments {
        return new_enchantments > current_enchantments;
    }
    let new_damage = new_item.get_damage();
    let current_damage = current_item.get_damage();
    if new_damage != current_damage {
        return new_damage < current_damage;
    }
    new_item.get_data_component::<CustomNameImpl>().is_some()
        && current_item
            .get_data_component::<CustomNameImpl>()
            .is_none()
}

/// Vanilla `Mob.equipItemIfPossible`: equips `stack` if it beats what is worn, returning
/// what got equipped.
#[must_use]
pub fn equip_item_if_possible<M: Mob + ?Sized>(mob: &M, stack: ItemStack) -> ItemStack {
    let mob_entity = mob.get_mob_entity();
    let mut slot = get_equipment_slot_for_item(mob, &stack);
    if !mob.is_equippable_in_slot(&stack, &slot) {
        return ItemStack::EMPTY.clone();
    }

    let mut current = mob_entity.get_item_slot(&slot);
    let mut can_replace = mob.can_replace_current_item(&stack, &current, &slot);
    if slot.is_armor_slot() && !can_replace {
        slot = EquipmentSlot::MAIN_HAND;
        current = mob_entity.get_item_slot(&slot);
        can_replace = current.is_empty();
    }
    if !can_replace || !mob.can_hold_item(&stack) {
        return ItemStack::EMPTY.clone();
    }

    let drop_chance = mob_entity.drop_chance(&slot);
    if !current.is_empty()
        && (mob_entity.living_entity.entity.random().next_f32() - 0.1).max(0.0) < drop_chance
    {
        mob_entity.living_entity.entity.spawn_at_location(current);
    }

    let mut stack = stack;
    let to_equip = limit_for_slot(&slot, &mut stack);
    // Vanilla `setItemSlotAndDropWhenKilled`, through `setItemSlot` and so `onEquipItem`.
    mob.set_item_slot_and_notify(&slot, to_equip.clone());
    mob_entity.set_guaranteed_drop(&slot);
    mob_entity
        .persistence_required
        .store(true, std::sync::atomic::Ordering::Relaxed);
    to_equip
}

/// The server side of vanilla `LivingEntity.onEquipItem`: the equip sound and the
/// `equip`/`unequip` game event when `slot` changed from `old` to `new`.
pub fn notify_equip<M: Mob + ?Sized>(
    mob: &M,
    slot: &EquipmentSlot,
    old: &ItemStack,
    new: &ItemStack,
) {
    let entity = mob.get_entity();
    // Vanilla `firstTick`: nothing sounds before the entity's first tick.
    if old.are_items_and_components_equal(new) || entity.tick_count.load(Relaxed) == 0 {
        return;
    }
    let world = entity.world.load();
    let pos = entity.pos.load();
    let equippable = new.get_data_component::<EquippableImpl>();
    if let Some(equippable) = equippable
        && !entity.is_silent()
        && equippable.slot == slot
    {
        let sound = mob
            .equip_sound(slot)
            .map_or_else(|| equippable.equip_sound.clone(), IdOr::Id);
        // `playSeededSound` with `this.random.nextLong()`.
        let seed = entity.random().next_i64();
        world.play_seeded_sound_event(&sound, mob.sound_source(), &pos, seed);
    }
    let event = if equippable.is_some() {
        GameEvent::Equip
    } else {
        GameEvent::Unequip
    };
    // `this.gameEvent(...)`: the mob is the source; `M: ?Sized` can't coerce, so look it up.
    let source = world.get_entity_by_id(entity.entity_id);
    world.game_event_from(source.as_deref(), event, pos);
}

// Hand slots have no count limit, every other slot holds one item.
fn limit_for_slot(slot: &EquipmentSlot, stack: &mut ItemStack) -> ItemStack {
    if slot.slot_type() == EquipmentType::Hand {
        std::mem::replace(stack, ItemStack::EMPTY.clone())
    } else {
        stack.split(1)
    }
}

/// Vanilla `Mob.pickUpItem`.
pub fn pick_up_item<M: Mob + ?Sized>(mob: &M, item: &crate::entity::item::ItemEntity) {
    let stack = item
        .get_item_stack()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let equipped = equip_item_if_possible(mob, stack);
    if equipped.is_empty() {
        return;
    }
    let count = equipped.item_count;
    // `take` tells the clients before the item goes.
    if !mob
        .get_mob_entity()
        .living_entity
        .pickup(item.get_entity(), u32::from(count))
    {
        return;
    }
    let emptied = {
        let mut item_stack = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        item_stack.decrement(count);
        item_stack.is_empty()
    };
    if emptied {
        item.get_entity().remove();
    } else {
        crate::entity::EntityBase::init_data_tracker(item);
    }
}

#[cfg(test)]
mod tests {
    use super::moon_brightness;

    #[test]
    fn attribute_modifiers_compute_in_order() {
        use pumpkin_data::AttributeModifierSlot;
        use pumpkin_data::attributes::Attributes;
        use pumpkin_data::data_component_impl::{
            AttributeModifiersImpl, EquipmentSlot, Modifier, Operation,
        };
        use std::borrow::Cow;

        let modifier = |operation, amount| Modifier {
            r#type: &Attributes::ARMOR,
            id: "test",
            amount,
            operation,
            slot: AttributeModifierSlot::Any,
        };
        let modifiers = AttributeModifiersImpl {
            attribute_modifiers: Cow::Owned(vec![
                modifier(Operation::AddMultipliedTotal, 0.5),
                modifier(Operation::AddValue, 1.0),
                modifier(Operation::AddMultipliedBase, 1.0),
            ]),
        };
        // 2 + 0.5 * 2 = 3, + 1 = 4, + 1 * 2 = 6: a total multiplier only scales what came
        // before it, and a base one the base.
        let value = super::compute_attribute_modifiers(
            &modifiers,
            &Attributes::ARMOR,
            2.0,
            &EquipmentSlot::HEAD,
        );
        assert!((value - 6.0).abs() < f64::EPSILON, "got {value}");
    }

    #[test]
    fn moon_brightness_matches_the_vanilla_phase_table() {
        let expected = [1.0, 0.75, 0.5, 0.25, 0.0, 0.25, 0.5, 0.75];
        for (phase, want) in expected.into_iter().enumerate() {
            let time_of_day = phase as i64 * 24000;
            assert!(
                (moon_brightness(time_of_day) - want).abs() < f32::EPSILON,
                "phase {phase}: got {}, want {want}",
                moon_brightness(time_of_day)
            );
        }
    }

    #[test]
    fn moon_brightness_wraps_every_eight_days() {
        for phase in 0..8i64 {
            assert!(
                (moon_brightness(phase * 24000) - moon_brightness((phase + 8) * 24000)).abs()
                    < f32::EPSILON,
                "phase {phase} does not wrap"
            );
        }
    }

    #[test]
    fn mob_spawn_equipment_enchantment_provider_applies_enchantments() {
        use pumpkin_data::data_component_impl::EquipmentSlot;
        use pumpkin_data::item::Item;
        use pumpkin_data::item_stack::ItemStack;

        let mut stack = ItemStack::new(1, &Item::DIAMOND_SWORD);
        super::apply_vanilla_enchantments(&mut stack, &EquipmentSlot::MAIN_HAND, 1.0);
        assert!(stack.has_enchantments());
    }

    #[test]
    fn pillager_crossbow_enchantments() {
        use pumpkin_data::Enchantment;
        use pumpkin_data::item::Item;
        use pumpkin_data::item_stack::ItemStack;

        let mut stack = ItemStack::new(1, &Item::CROSSBOW);
        super::apply_pillager_crossbow_enchantments(&mut stack, None);
        assert_eq!(stack.get_enchantment_level(&Enchantment::PIERCING), 1);

        let mut stack3 = ItemStack::new(1, &Item::CROSSBOW);
        super::apply_pillager_crossbow_enchantments(&mut stack3, Some(3));
        assert_eq!(stack3.get_enchantment_level(&Enchantment::QUICK_CHARGE), 1);

        let mut stack5 = ItemStack::new(1, &Item::CROSSBOW);
        super::apply_pillager_crossbow_enchantments(&mut stack5, Some(5));
        assert_eq!(stack5.get_enchantment_level(&Enchantment::QUICK_CHARGE), 2);
    }

    #[test]
    fn vindicator_weapon_enchantments() {
        use pumpkin_data::Enchantment;
        use pumpkin_data::item::Item;
        use pumpkin_data::item_stack::ItemStack;

        let mut stack = ItemStack::new(1, &Item::IRON_AXE);
        super::apply_vindicator_weapon_enchantments(&mut stack, Some(1));
        assert_eq!(stack.get_enchantment_level(&Enchantment::SHARPNESS), 1);

        let mut stack5 = ItemStack::new(1, &Item::IRON_AXE);
        super::apply_vindicator_weapon_enchantments(&mut stack5, Some(5));
        assert_eq!(stack5.get_enchantment_level(&Enchantment::SHARPNESS), 2);
    }
}
