//! Mob equipment: vanilla `DifficultyInstance` for spawn equipment and `Mob`'s equipment rules
//! (`equipItemIfPossible`, `canReplaceCurrentItem`, `pickUpItem`).

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::{
    AttributeModifiersImpl, CustomNameImpl, EnchantmentsImpl, EquipmentSlot, EquipmentType,
    EquippableImpl, IDSet, IdOr, Operation,
};
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
    /// Vanilla `ServerLevel.getCurrentDifficultyAt`: the chunk's inhabited time and the moon
    /// only count when the chunk is loaded; the time is the overworld clock's.
    pub fn at(world: &Arc<crate::world::World>, pos: Vector3<f64>) -> Self {
        let difficulty = world.level_info.load().difficulty;
        let block_pos = pos.to_block_pos();
        let chunk = world
            .level
            .loaded_chunks
            .get(&pumpkin_util::math::vector2::Vector2::new(
                block_pos.0.x >> 4,
                block_pos.0.z >> 4,
            ))
            .map(|chunk| chunk.inhabited_time.load(Relaxed));
        let (inhabited_time, moon_brightness) = chunk.map_or((0, 0.0), |inhabited_time| {
            (inhabited_time, world.get_moon_brightness(&block_pos))
        });

        Self::calculate(
            difficulty,
            world.get_overworld_clock_time(),
            inhabited_time,
            moon_brightness,
        )
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

/// Where vanilla `Mob.equipItemIfPossible` would put a stack: the slot, what is worn there and
/// how many items `slot.limit` takes.
pub(crate) struct EquipDecision {
    slot: EquipmentSlot,
    current: ItemStack,
    count: u8,
}

impl EquipDecision {
    /// How many items the slot takes: the whole stack in a hand, one anywhere else.
    pub(crate) const fn count(&self) -> u8 {
        self.count
    }
}

/// Vanilla `Mob.equipItemIfPossible` up to `canReplace && canHoldItem`: no draws, no changes.
pub(crate) fn equip_decision<M: Mob + ?Sized>(mob: &M, stack: &ItemStack) -> Option<EquipDecision> {
    let mob_entity = mob.get_mob_entity();
    let mut slot = get_equipment_slot_for_item(mob, stack);
    if !mob.is_equippable_in_slot(stack, &slot) {
        return None;
    }

    let mut current = mob_entity.get_item_slot(&slot);
    let mut can_replace = mob.can_replace_current_item(stack, &current, &slot);
    if slot.is_armor_slot() && !can_replace {
        slot = EquipmentSlot::MAIN_HAND;
        current = mob_entity.get_item_slot(&slot);
        can_replace = current.is_empty();
    }
    if !can_replace || !mob.can_hold_item(stack) {
        return None;
    }
    let count = if slot.slot_type() == EquipmentType::Hand {
        stack.item_count
    } else {
        stack.item_count.min(1)
    };
    Some(EquipDecision {
        slot,
        current,
        count,
    })
}

/// The rest of vanilla `Mob.equipItemIfPossible`: the drop-chance draw for what is worn, then
/// `slot.limit(stack)` equipped for good. Returns what got equipped.
pub(crate) fn equip_commit<M: Mob + ?Sized>(
    mob: &M,
    decision: EquipDecision,
    mut stack: ItemStack,
) -> ItemStack {
    let mob_entity = mob.get_mob_entity();
    let EquipDecision { slot, current, .. } = decision;
    let drop_chance = mob_entity.drop_chance(&slot);
    if !current.is_empty()
        && (mob_entity.living_entity.entity.random().next_f32() - 0.1).max(0.0) < drop_chance
    {
        mob_entity.living_entity.entity.spawn_at_location(current);
    }

    let to_equip = limit_for_slot(&slot, &mut stack);
    // Vanilla `setItemSlotAndDropWhenKilled`, through `setItemSlot` and so `onEquipItem`.
    mob.set_item_slot_and_notify(&slot, to_equip.clone());
    mob_entity.set_guaranteed_drop(&slot);
    mob_entity
        .persistence_required
        .store(true, std::sync::atomic::Ordering::Relaxed);
    to_equip
}

/// Vanilla `Mob.equipItemIfPossible`: equips `stack` if it beats what is worn, returning
/// what got equipped.
#[must_use]
pub fn equip_item_if_possible<M: Mob + ?Sized>(mob: &M, stack: ItemStack) -> ItemStack {
    equip_decision(mob, &stack).map_or_else(
        || ItemStack::EMPTY.clone(),
        |decision| equip_commit(mob, decision, stack),
    )
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

/// Vanilla `Mob.pickUpItem`. Pumpkin's pickup event goes between deciding the slot and
/// equipping, so a cancelled pickup changes nothing and draws nothing.
pub fn pick_up_item<M: Mob + ?Sized>(mob: &M, item: &crate::entity::item::ItemEntity) {
    let stack = item
        .get_item_stack()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let Some(decision) = equip_decision(mob, &stack) else {
        return;
    };
    let living = &mob.get_mob_entity().living_entity;
    if !living.fire_pickup_event(item.get_entity(), u32::from(decision.count())) {
        return;
    }
    let count = equip_commit(mob, decision, stack).item_count;
    living.on_item_pickup(item);
    living.take(item.get_entity(), u32::from(count));
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
    use super::*;

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

    /// `getCurrentDifficultyAt` with the chunk unloaded: no inhabited time and no moon. Day 63
    /// on hard is `3 * (0.75 + 0.25)`; the waxing gibbous moon would add `3 * 0.1875`.
    #[tokio::test]
    async fn unloaded_chunk_has_no_moon_or_inhabited_time() {
        let (_folder, world) = crate::world::test_support::test_world();
        world.set_time_of_day(1_512_000);
        let mut level_info = (**world.level_info.load()).clone();
        level_info.difficulty = Difficulty::Hard;
        world.level_info.store(Arc::new(level_info));
        let difficulty = RegionalDifficulty::at(&world, Vector3::new(8.0, 64.0, 8.0));
        assert!(
            (difficulty.effective_difficulty - 3.0).abs() < f32::EPSILON,
            "got {}",
            difficulty.effective_difficulty
        );
    }
}
