use crate::entity::EntityBase;
use pumpkin_data::BlockState;
use pumpkin_data::Enchantment;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::EnchantmentsImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::recipes::{CookingRecipeKind, get_cooking_recipe_with_ingredient};
use pumpkin_data::tag::Taggable;
use pumpkin_util::identifier::Identifier;
pub use pumpkin_util::loot_table::{
    DynamicLootCondition, DynamicLootEntry, DynamicLootEntryKind, DynamicLootFunction,
    DynamicLootPool, DynamicLootTable, LootBonusFormula, LootCondition, LootEntityTarget,
    LootEntry, LootEntryKind, LootFunction, LootFunctionKind, LootNumberProvider, LootPool,
    LootTable,
};
use pumpkin_util::math::java_round_f32;
use pumpkin_util::random::legacy_rand::LegacyRand;
use pumpkin_util::random::{RandomImpl, shuffle};

use crate::world::World;

/// Coerce `&EntityType` to `&dyn EntityBase` for `LootContextParameters`.
/// `EntityType` implements `EntityBase` but Rust can't double-coerce through `Some()`.
fn as_entity_base(entity_type: &pumpkin_data::entity::EntityType) -> &dyn EntityBase {
    entity_type
}

mod predicates;
mod tests;

#[derive(Default, Clone)]
pub struct LootContextParameters<'a> {
    pub explosion_radius: Option<f32>,
    pub block_state: Option<&'a BlockState>,
    pub killed_by_player: Option<bool>,
    pub luck: f32,
    pub this_entity: Option<&'a dyn EntityBase>,
    pub killer_entity: Option<&'a dyn EntityBase>,
    /// Vanilla `EnchantmentHelper.getEnchantmentLevel(LOOTING, ATTACKING_ENTITY)`: 0 unless the
    /// attacker is a living entity with Looting in a slot Looting works from.
    pub attacking_entity_looting_level: i32,
    pub direct_killer_entity: Option<&'a dyn EntityBase>,
    pub position: Option<pumpkin_util::math::vector3::Vector3<f64>>,
    pub world_time: u64,
    pub damage_type: Option<DamageType>,
    pub tool: Option<ItemStack>,
    pub is_raining: Option<bool>,
    pub is_thundering: Option<bool>,
    /// Whether the killed entity was on fire at death time.
    /// Computed from `Entity.fire_ticks > 0`.
    pub is_on_fire: Option<bool>,
    pub direct_attacker_weapon: Option<ItemStack>,
    /// Vanilla `DamageSource.getEntity`'s type, for `damage_source_properties`.
    pub damage_source_entity: Option<&'a dyn EntityBase>,
    /// The `minecraft:frog/variant` of that entity, when it is a frog.
    pub damage_source_frog_variant: Option<&'a str>,
    /// The size of the killed entity, when it is a slime or magma cube.
    pub this_cube_mob_size: Option<i32>,
    /// Reference to the world for location checks.
    pub world: Option<&'a World>,
}

/// Vanilla `DamageSourcePredicate.sourceEntity` on its type and frog variant.
fn damage_source_entity_matches(
    params: &LootContextParameters<'_>,
    entity_type: &str,
    frog_variant: Option<&str>,
) -> bool {
    let entity_type = entity_type
        .strip_prefix("minecraft:")
        .unwrap_or(entity_type);
    params
        .damage_source_entity
        .is_some_and(|source| source.get_entity().entity_type.resource_name == entity_type)
        && frog_variant.is_none_or(|variant| params.damage_source_frog_variant == Some(variant))
}

/// Vanilla `EntityTypePredicate` on `this`.
fn this_entity_type_matches(params: &LootContextParameters<'_>, entity_type: &str) -> bool {
    let entity_type = entity_type
        .strip_prefix("minecraft:")
        .unwrap_or(entity_type);
    params
        .this_entity
        .is_some_and(|this| this.get_entity().entity_type.resource_name == entity_type)
}

/// Vanilla `CubeMobPredicate`'s size range.
fn this_cube_mob_size_matches(params: &LootContextParameters<'_>, min: i32, max: i32) -> bool {
    params
        .this_cube_mob_size
        .is_some_and(|size| (min..=max).contains(&size))
}

fn direct_attacker_weapon_has_enchantment(params: &LootContextParameters<'_>, tag: &str) -> bool {
    params
        .direct_attacker_weapon
        .as_ref()
        .and_then(|weapon| weapon.get_data_component::<EnchantmentsImpl>())
        .is_some_and(|enchantments| {
            enchantments
                .enchantment
                .iter()
                .any(|(enchantment, _)| enchantment.is_tagged_with(tag) == Some(true))
        })
}

fn smelt(stack: ItemStack) -> ItemStack {
    let result = get_cooking_recipe_with_ingredient(stack.item, CookingRecipeKind::Smelting)
        .and_then(|recipe| {
            let id = recipe
                .result
                .id
                .strip_prefix("minecraft:")
                .unwrap_or(recipe.result.id);
            Item::from_registry_key(id).map(|item| (item, recipe.result.count))
        });
    let Some((item, count)) = result else {
        tracing::warn!(
            "Couldn't smelt {} because there is no smelting recipe",
            stack.item.registry_key
        );
        return stack;
    };
    let mut result = ItemStack::new(1, item);
    let count = (u32::from(stack.item_count) * u32::from(count))
        .min(u32::from(result.get_max_stack_size()));
    result.item_count = count as u8;
    result
}

/// Evaluate a `LootNumberProvider` to get a roll count.
fn evaluate_rolls<R: RandomImpl>(provider: &LootNumberProvider, rng: &mut R) -> i32 {
    match provider {
        LootNumberProvider::Constant(v) => *v as i32,
        LootNumberProvider::Uniform { min, max } => {
            rng.next_inbetween_i32(*min as i32, *max as i32)
        }
        LootNumberProvider::Binomial { n, probability } => {
            let mut count = 0;
            for _ in 0..*n {
                if rng.next_f32() < *probability {
                    count += 1;
                }
            }
            count
        }
    }
}

/// Evaluate a `LootFunction` that modifies count.
fn apply_count_function<R: RandomImpl>(
    base_count: i32,
    function: &LootFunction,
    fortune_level: i32,
    looting_level: i32,
    params: &LootContextParameters<'_>,
    rng: &mut R,
) -> i32 {
    match &function.kind {
        LootFunctionKind::SetCount {
            count: provider,
            add,
        } => {
            let new_count = evaluate_rolls(provider, rng);
            if *add {
                base_count + new_count
            } else {
                new_count
            }
        }
        LootFunctionKind::EnchantedCountIncrease { count, limit } => {
            let level = looting_level;
            if level == 0 {
                return base_count;
            }
            let bonus = evaluate_rolls(count, rng);
            let result =
                base_count + java_round_f32(level as f32 * rng.mth_next_float(0.0, 1.0)) + bonus;
            limit.map_or(result, |lim| result.min(lim))
        }
        LootFunctionKind::LimitCount { min, max } => {
            let mut result = base_count;
            if let Some(min_p) = min {
                let min_val = evaluate_rolls(min_p, rng);
                result = result.max(min_val);
            }
            if let Some(max_p) = max {
                let max_val = evaluate_rolls(max_p, rng);
                result = result.min(max_val);
            }
            result
        }
        _ => base_count,
    }
}

/// Extract the item name from a `LootEntryKind`.
fn entry_item_name(entry: &LootEntry) -> Option<&'static str> {
    match entry.kind {
        LootEntryKind::Item(name) => Some(name),
        _ => None,
    }
}

/// Get the effective count for an entry after applying its functions.
fn entry_count<R: RandomImpl>(
    entry: &LootEntry,
    fortune_level: i32,
    looting_level: i32,
    params: &LootContextParameters<'_>,
    rng: &mut R,
) -> Option<i32> {
    // Start with a default count of 1 (the upstream model doesn't have min/max count fields;
    // count is set via `set_count` function).
    let mut count: i32 = 1;

    // Apply entry functions
    for func in entry.functions {
        count = apply_count_function(count, func, fortune_level, looting_level, params, rng);
    }

    // Apply table-level functions (already applied in pool processing)
    Some(count)
}

/// Evaluate a static `LootCondition` against the context.
/// For new condition types the fork doesn't understand yet, we return true to preserve
/// the table structure (upstream's approach). The fork's original conditions still
/// use their dedicated logic below.
fn check_condition<R: RandomImpl>(
    cond: LootCondition,
    has_silk_touch: bool,
    has_shears: bool,
    fortune_level: i32,
    looting_level: i32,
    params: &LootContextParameters<'_>,
    rng: &mut R,
) -> bool {
    match cond {
        LootCondition::None | LootCondition::Never => true,
        LootCondition::SilkTouch => has_silk_touch,
        LootCondition::NoSilkTouch => !has_silk_touch,
        LootCondition::Shears => has_shears,
        LootCondition::SilkTouchOrShears => has_silk_touch || has_shears,
        LootCondition::NoSilkTouchOrShears => !has_silk_touch && !has_shears,
        LootCondition::KilledByPlayer => params.killed_by_player.unwrap_or(false),
        LootCondition::SurvivesExplosion => params
            .explosion_radius
            .is_none_or(|radius| rng.next_f32() <= 1.0 / radius),
        LootCondition::RandomChance { chance } => rng.next_f32() < chance,
        LootCondition::RandomChanceWithEnchantedBonus {
            enchanted_chance_base,
            enchanted_chance_per_level_above_first,
            ..
        } => {
            let level = looting_level;
            let chance = if level > 0 {
                enchanted_chance_base + enchanted_chance_per_level_above_first * (level - 1) as f32
            } else {
                0.0 // unenchanted_chance defaults to 0 when not specified
            };
            rng.next_f32() < chance
        }
        LootCondition::TableBonus { chances, .. } => {
            let index = (fortune_level.max(0) as usize).min(chances.len().saturating_sub(1));
            chances
                .get(index)
                .is_some_and(|chance| rng.next_f32() < *chance)
        }
        LootCondition::AllOf(conditions) => conditions.iter().all(|c| {
            check_condition(
                *c,
                has_silk_touch,
                has_shears,
                fortune_level,
                looting_level,
                params,
                rng,
            )
        }),
        LootCondition::AnyOf(conditions) => conditions.iter().any(|c| {
            check_condition(
                *c,
                has_silk_touch,
                has_shears,
                fortune_level,
                looting_level,
                params,
                rng,
            )
        }),
        LootCondition::Inverted(cond) => !check_condition(
            *cond,
            has_silk_touch,
            has_shears,
            fortune_level,
            looting_level,
            params,
            rng,
        ),
        LootCondition::EntityOnFire => params.is_on_fire.unwrap_or(false),
        LootCondition::DirectAttackerWeaponEnchantment(tag) => {
            direct_attacker_weapon_has_enchantment(params, tag)
        }
        // New condition types from upstream: evaluate via predicates module or pass through.
        LootCondition::EntityProperties { target, predicate } => {
            predicates::matches_entity(target, &*predicate, params)
        }
        LootCondition::MatchBlock(predicate) => predicates::matches_block(&*predicate, params),
        LootCondition::LocationCheck { offset, predicate } => {
            predicates::matches_location(*offset, &*predicate, params)
        }
        LootCondition::MatchTool(predicate) => predicates::matches_tool(&*predicate, params),
        LootCondition::WeatherCheck {
            raining,
            thundering,
        } => {
            if let Some(r) = raining
                && params.is_raining != Some(*r)
            {
                return false;
            }
            if let Some(t) = thundering
                && params.is_thundering != Some(*t)
            {
                return false;
            }
            true
        }
        LootCondition::DamageSourceTag { tag, expected } => {
            // Simplified: check if the damage type matches the tag
            // Full implementation would need damage type tag resolution
            params.damage_type.as_ref().map_or(expected, |dt| {
                // Tag match is handled at the datapack level; for now,
                // we treat unknown tags as not matching when expected=true
                expected
            })
        }
    }
}

fn pick_entry<'a, R: RandomImpl>(
    valid_entries: &[&'a LootEntry],
    random: &mut R,
) -> Option<&'a LootEntry> {
    let total_weight: i32 = valid_entries.iter().map(|entry| entry.weight).sum();
    if total_weight == 0 {
        return None;
    }
    if let [entry] = valid_entries {
        return Some(entry);
    }
    let mut index = random.next_bounded_i32(total_weight);
    valid_entries.iter().copied().find(|entry| {
        index -= entry.weight;
        index < 0
    })
}

fn evaluate_static<R: RandomImpl>(
    table: &LootTable,
    random: &mut R,
    params: &LootContextParameters<'_>,
) -> Vec<ItemStack> {
    let mut items_to_place: Vec<ItemStack> = Vec::new();
    let has_silk_touch = tool_has_silk_touch(params);
    let has_shears = tool_is_shears(params);
    let fortune_level = tool_fortune_level(params);
    let looting_level = params.attacking_entity_looting_level;

    // Vanilla `LootPool.addRandomItems`
    for pool in table.pools {
        if !check_condition(
            pool.condition,
            has_silk_touch,
            has_shears,
            fortune_level,
            looting_level,
            params,
            random,
        ) {
            continue;
        }

        let rolls = evaluate_rolls(&pool.rolls, random);
        let bonus_rolls = evaluate_rolls(&pool.bonus_rolls, random);
        let total_rolls = rolls + bonus_rolls;

        for _ in 0..total_rolls {
            // Each roll expands the entries again, testing their conditions.
            let mut valid_entries: Vec<&LootEntry> = Vec::new();
            for entry in pool.entries {
                if check_condition(
                    entry.condition,
                    has_silk_touch,
                    has_shears,
                    fortune_level,
                    looting_level,
                    params,
                    random,
                ) && entry.weight > 0
                {
                    valid_entries.push(entry);
                }
            }
            let Some(entry) = pick_entry(&valid_entries, random) else {
                continue;
            };

            // Handle different entry kinds
            let item_name = match &entry.kind {
                LootEntryKind::Item(name) => Some(*name),
                LootEntryKind::Empty => None,
                LootEntryKind::Dynamic(_) => {
                    // Dynamic entries reference datapack tables - skip in static evaluation
                    continue;
                }
                LootEntryKind::Tag { .. } => {
                    // Tag entries expand at codegen time
                    continue;
                }
                LootEntryKind::Alternatives(children) => {
                    // Pick the first matching child
                    for child in *children {
                        if check_condition(
                            child.condition,
                            has_silk_touch,
                            has_shears,
                            fortune_level,
                            looting_level,
                            params,
                            random,
                        ) && child.weight > 0
                        {
                            // Use child's item
                            match &child.kind {
                                LootEntryKind::Item(name) => {
                                    return evaluate_entry(
                                        name,
                                        child,
                                        has_silk_touch,
                                        has_shears,
                                        fortune_level,
                                        looting_level,
                                        params,
                                        random,
                                    );
                                }
                                _ => continue,
                            }
                        }
                    }
                    continue;
                }
                LootEntryKind::Group(children) => {
                    // Group entries all succeed in order
                    for child in *children {
                        evaluate_entry_recursive(
                            child,
                            has_silk_touch,
                            has_shears,
                            fortune_level,
                            looting_level,
                            params,
                            random,
                            &mut items_to_place,
                        );
                    }
                    continue;
                }
                LootEntryKind::Sequence(children) => {
                    // Sequence entries succeed in order until one fails
                    for child in *children {
                        if check_condition(
                            child.condition,
                            has_silk_touch,
                            has_shears,
                            fortune_level,
                            looting_level,
                            params,
                            random,
                        ) {
                            evaluate_entry_recursive(
                                child,
                                has_silk_touch,
                                has_shears,
                                fortune_level,
                                looting_level,
                                params,
                                random,
                                &mut items_to_place,
                            );
                            break;
                        }
                    }
                    continue;
                }
                LootEntryKind::Table(key) => {
                    // Reference to another loot table - skip in static evaluation
                    continue;
                }
                LootEntryKind::InlineTable(inline_table) => {
                    // Inline table - evaluate it
                    let inline_items = evaluate_static(inline_table, random, params);
                    items_to_place.extend(inline_items);
                    continue;
                }
            };

            if let Some(item_name) = item_name {
                let result = evaluate_entry(
                    item_name,
                    entry,
                    has_silk_touch,
                    has_shears,
                    fortune_level,
                    looting_level,
                    params,
                    random,
                );
                items_to_place.extend(result);
            }
        }
    }

    items_to_place
}

/// Evaluate a single entry and return its items.
fn evaluate_entry<R: RandomImpl>(
    item_name: &str,
    entry: &LootEntry,
    has_silk_touch: bool,
    has_shears: bool,
    fortune_level: i32,
    looting_level: i32,
    params: &LootContextParameters<'_>,
    random: &mut R,
) -> Vec<ItemStack> {
    let item = Item::from_registry_key(item_name.strip_prefix("minecraft:").unwrap_or(item_name));
    let Some(item) = item else {
        return Vec::new();
    };

    // Start with default count of 1
    let mut count: i32 = 1;

    // Apply entry-level functions
    for func in entry.functions {
        count = apply_count_function(count, func, fortune_level, looting_level, params, random);
    }

    // Check for furnace_smelt function
    let smelts = entry
        .functions
        .iter()
        .any(|f| matches!(f.kind, LootFunctionKind::FurnaceSmelt));

    if count <= 0 {
        return Vec::new();
    }

    let stack = ItemStack::new(1, item);
    if smelts {
        let smelted = smelt(stack);
        split_stack(&smelted, i32::from(smelted.item_count), &mut Vec::new());
        // For smelted items, we just add the smelted stack
        vec![smelted]
    } else {
        split_stack(&stack, count, &mut Vec::new());
        // Return individual stacks
        let mut result = Vec::new();
        let max_size = i32::from(stack.get_max_stack_size());
        let mut remaining = count;
        while remaining > 0 {
            let piece = remaining.min(max_size) as u8;
            result.push(ItemStack::new(piece, item));
            remaining -= piece as i32;
        }
        result
    }
}

/// Recursively evaluate an entry for group/sequence handling.
fn evaluate_entry_recursive<R: RandomImpl>(
    entry: &LootEntry,
    has_silk_touch: bool,
    has_shears: bool,
    fortune_level: i32,
    looting_level: i32,
    params: &LootContextParameters<'_>,
    random: &mut R,
    output: &mut Vec<ItemStack>,
) {
    let item_name = match &entry.kind {
        LootEntryKind::Item(name) => Some(*name),
        _ => None,
    };

    if let Some(item_name) = item_name {
        let item =
            Item::from_registry_key(item_name.strip_prefix("minecraft:").unwrap_or(item_name));
        if let Some(item) = item {
            let mut count: i32 = 1;
            for func in entry.functions {
                count =
                    apply_count_function(count, func, fortune_level, looting_level, params, random);
            }
            if count > 0 {
                let stack = ItemStack::new(1, item);
                split_stack(&stack, count, output);
            }
        }
    }
}

fn evaluate_dynamic<R: RandomImpl>(
    table: &DynamicLootTable,
    random: &mut R,
    params: &LootContextParameters<'_>,
) -> Vec<ItemStack> {
    let mut items_to_place: Vec<ItemStack> = Vec::new();
    let has_silk_touch = tool_has_silk_touch(params);
    let has_shears = tool_is_shears(params);
    let fortune_level = tool_fortune_level(params);
    let looting_level = params.attacking_entity_looting_level;

    for pool in &table.pools {
        if !check_dynamic_condition(
            &pool.condition,
            has_silk_touch,
            has_shears,
            fortune_level,
            looting_level,
            params,
            random,
        ) {
            continue;
        }

        let rolls = evaluate_rolls(&pool.rolls, random);
        let bonus_rolls = evaluate_rolls(&pool.bonus_rolls, random);
        let total_rolls = rolls + bonus_rolls;

        for _ in 0..total_rolls {
            let mut valid_entries: Vec<&DynamicLootEntry> = Vec::new();
            for entry in &pool.entries {
                if check_dynamic_condition(
                    &entry.condition,
                    has_silk_touch,
                    has_shears,
                    fortune_level,
                    looting_level,
                    params,
                    random,
                ) && entry.weight > 0
                {
                    valid_entries.push(entry);
                }
            }
            let Some(entry) = pick_dynamic_entry(&valid_entries, random) else {
                continue;
            };

            // Handle different entry kinds
            match &entry.kind {
                DynamicLootEntryKind::Item(item_name) => {
                    let item = Item::from_registry_key(
                        item_name.strip_prefix("minecraft:").unwrap_or(item_name),
                    );
                    if let Some(item) = item {
                        let mut count: i32 = 1;
                        for func in &entry.functions {
                            count = apply_dynamic_count_function(
                                count,
                                func,
                                fortune_level,
                                looting_level,
                                params,
                                random,
                            );
                        }
                        if count > 0 {
                            let stack = ItemStack::new(1, item);
                            split_stack(&stack, count, &mut items_to_place);
                        }
                    }
                }
                DynamicLootEntryKind::Empty => {}
                DynamicLootEntryKind::Dynamic(_) => continue,
                DynamicLootEntryKind::Tag { .. } => continue,
                DynamicLootEntryKind::Alternatives(children) => {
                    for child in children {
                        if check_dynamic_condition(
                            &child.condition,
                            has_silk_touch,
                            has_shears,
                            fortune_level,
                            looting_level,
                            params,
                            random,
                        ) && child.weight > 0
                        {
                            if let DynamicLootEntryKind::Item(name) = &child.kind {
                                let item = Item::from_registry_key(
                                    name.strip_prefix("minecraft:").unwrap_or(name),
                                );
                                if let Some(item) = item {
                                    let mut count: i32 = 1;
                                    for func in &child.functions {
                                        count = apply_dynamic_count_function(
                                            count,
                                            func,
                                            fortune_level,
                                            looting_level,
                                            params,
                                            random,
                                        );
                                    }
                                    if count > 0 {
                                        let stack = ItemStack::new(1, item);
                                        split_stack(&stack, count, &mut items_to_place);
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
                DynamicLootEntryKind::Group(children) => {
                    for child in children {
                        evaluate_dynamic_entry_recursive(
                            child,
                            has_silk_touch,
                            has_shears,
                            fortune_level,
                            looting_level,
                            params,
                            random,
                            &mut items_to_place,
                        );
                    }
                }
                DynamicLootEntryKind::Sequence(children) => {
                    for child in children {
                        if check_dynamic_condition(
                            &child.condition,
                            has_silk_touch,
                            has_shears,
                            fortune_level,
                            looting_level,
                            params,
                            random,
                        ) {
                            evaluate_dynamic_entry_recursive(
                                child,
                                has_silk_touch,
                                has_shears,
                                fortune_level,
                                looting_level,
                                params,
                                random,
                                &mut items_to_place,
                            );
                            break;
                        }
                    }
                }
                DynamicLootEntryKind::Table(_) => continue,
                DynamicLootEntryKind::InlineTable(inline_table) => {
                    let inline_items = evaluate_dynamic(inline_table, random, params);
                    items_to_place.extend(inline_items);
                }
            }
        }
    }

    items_to_place
}

fn pick_dynamic_entry<'a, R: RandomImpl>(
    valid_entries: &[&'a DynamicLootEntry],
    random: &mut R,
) -> Option<&'a DynamicLootEntry> {
    let total_weight: i32 = valid_entries.iter().map(|entry| entry.weight).sum();
    if total_weight == 0 {
        return None;
    }
    if let [entry] = valid_entries {
        return Some(entry);
    }
    let mut index = random.next_bounded_i32(total_weight);
    valid_entries.iter().copied().find(|entry| {
        index -= entry.weight;
        index < 0
    })
}

fn apply_dynamic_count_function<R: RandomImpl>(
    base_count: i32,
    function: &DynamicLootFunction,
    fortune_level: i32,
    looting_level: i32,
    params: &LootContextParameters<'_>,
    rng: &mut R,
) -> i32 {
    match &function.kind {
        LootFunctionKind::SetCount {
            count: provider,
            add,
        } => {
            let new_count = evaluate_rolls(provider, rng);
            if *add {
                base_count + new_count
            } else {
                new_count
            }
        }
        LootFunctionKind::EnchantedCountIncrease { count, limit } => {
            let level = looting_level;
            if level == 0 {
                return base_count;
            }
            let bonus = evaluate_rolls(count, rng);
            let result =
                base_count + java_round_f32(level as f32 * rng.mth_next_float(0.0, 1.0)) + bonus;
            limit.map_or(result, |lim| result.min(lim))
        }
        LootFunctionKind::LimitCount { min, max } => {
            let mut result = base_count;
            if let Some(min_p) = min {
                let min_val = evaluate_rolls(min_p, rng);
                result = result.max(min_val);
            }
            if let Some(max_p) = max {
                let max_val = evaluate_rolls(max_p, rng);
                result = result.min(max_val);
            }
            result
        }
        _ => base_count,
    }
}

fn evaluate_dynamic_entry_recursive<R: RandomImpl>(
    entry: &DynamicLootEntry,
    has_silk_touch: bool,
    has_shears: bool,
    fortune_level: i32,
    looting_level: i32,
    params: &LootContextParameters<'_>,
    random: &mut R,
    output: &mut Vec<ItemStack>,
) {
    let item_name = match &entry.kind {
        DynamicLootEntryKind::Item(name) => Some(name.as_str()),
        _ => None,
    };

    if let Some(item_name) = item_name {
        let item =
            Item::from_registry_key(item_name.strip_prefix("minecraft:").unwrap_or(item_name));
        if let Some(item) = item {
            let mut count: i32 = 1;
            for func in &entry.functions {
                count = apply_dynamic_count_function(
                    count,
                    func,
                    fortune_level,
                    looting_level,
                    params,
                    random,
                );
            }
            if count > 0 {
                let stack = ItemStack::new(1, item);
                split_stack(&stack, count, output);
            }
        }
    }
}

/// Check a dynamic condition against the context.
fn check_dynamic_condition<R: RandomImpl>(
    cond: &DynamicLootCondition,
    has_silk_touch: bool,
    has_shears: bool,
    fortune_level: i32,
    looting_level: i32,
    params: &LootContextParameters<'_>,
    rng: &mut R,
) -> bool {
    match cond {
        DynamicLootCondition::None | DynamicLootCondition::Never => true,
        DynamicLootCondition::SilkTouch => has_silk_touch,
        DynamicLootCondition::NoSilkTouch => !has_silk_touch,
        DynamicLootCondition::Shears => has_shears,
        DynamicLootCondition::SilkTouchOrShears => has_silk_touch || has_shears,
        DynamicLootCondition::NoSilkTouchOrShears => !has_silk_touch && !has_shears,
        DynamicLootCondition::KilledByPlayer => params.killed_by_player.unwrap_or(false),
        DynamicLootCondition::SurvivesExplosion => params
            .explosion_radius
            .is_none_or(|radius| rng.next_f32() <= 1.0 / radius),
        DynamicLootCondition::RandomChance { chance } => rng.next_f32() < *chance,
        DynamicLootCondition::RandomChanceWithEnchantedBonus {
            enchanted_chance_base,
            enchanted_chance_per_level_above_first,
            ..
        } => {
            let level = looting_level;
            let chance = if level > 0 {
                *enchanted_chance_base
                    + *enchanted_chance_per_level_above_first * (level - 1) as f32
            } else {
                0.0
            };
            rng.next_f32() < chance
        }
        DynamicLootCondition::TableBonus { chances, .. } => {
            let index = (fortune_level.max(0) as usize).min(chances.len().saturating_sub(1));
            chances
                .get(index)
                .is_some_and(|chance| rng.next_f32() < *chance)
        }
        DynamicLootCondition::AllOf(conditions) => conditions.iter().all(|c| {
            check_dynamic_condition(
                c,
                has_silk_touch,
                has_shears,
                fortune_level,
                looting_level,
                params,
                rng,
            )
        }),
        DynamicLootCondition::AnyOf(conditions) => conditions.iter().any(|c| {
            check_dynamic_condition(
                c,
                has_silk_touch,
                has_shears,
                fortune_level,
                looting_level,
                params,
                rng,
            )
        }),
        DynamicLootCondition::Inverted(cond) => !check_dynamic_condition(
            cond,
            has_silk_touch,
            has_shears,
            fortune_level,
            looting_level,
            params,
            rng,
        ),
        DynamicLootCondition::EntityOnFire => params.is_on_fire.unwrap_or(false),
        DynamicLootCondition::DirectAttackerWeaponEnchantment(tag) => {
            direct_attacker_weapon_has_enchantment(params, tag)
        }
        DynamicLootCondition::EntityProperties { target, predicate } => {
            predicates::matches_entity(*target, predicate, params)
        }
        DynamicLootCondition::MatchBlock(predicate) => predicates::matches_block(predicate, params),
        DynamicLootCondition::LocationCheck { offset, predicate } => {
            predicates::matches_location(*offset, predicate, params)
        }
        DynamicLootCondition::MatchTool(predicate) => predicates::matches_tool(predicate, params),
        DynamicLootCondition::WeatherCheck {
            raining,
            thundering,
        } => {
            if let Some(r) = raining
                && params.is_raining != Some(*r)
            {
                return false;
            }
            if let Some(t) = thundering
                && params.is_thundering != Some(*t)
            {
                return false;
            }
            true
        }
        DynamicLootCondition::DamageSourceTag { tag, expected } => {
            params.damage_type.as_ref().map_or(*expected, |dt| {
                // Simplified tag matching
                *expected
            })
        }
    }
}

/// The tool's Fortune level, which `apply_bonus` and `table_bonus` read.
fn tool_fortune_level(params: &LootContextParameters<'_>) -> i32 {
    params
        .tool
        .as_ref()
        .map_or(0, |tool| tool.get_enchantment_level(&Enchantment::FORTUNE))
}

fn tool_has_silk_touch(params: &LootContextParameters<'_>) -> bool {
    params
        .tool
        .as_ref()
        .is_some_and(|tool| tool.get_enchantment_level(&Enchantment::SILK_TOUCH) > 0)
}

fn tool_is_shears(params: &LootContextParameters<'_>) -> bool {
    params
        .tool
        .as_ref()
        .is_some_and(|tool| tool.item == &Item::SHEARS)
}

/// Vanilla `LootPool.addRandomItem`'s pick among the entries that expanded with a positive
/// weight: a lone entry is taken without a draw.
fn split_stack(stack: &ItemStack, count: i32, output: &mut Vec<ItemStack>) {
    if count <= 0 || stack.is_empty() {
        return;
    }
    let max = i32::from(stack.get_max_stack_size());
    let mut count = count;
    while count > 0 {
        let piece = count.min(max);
        output.push(stack.copy_with_count(piece as u8));
        count -= piece;
    }
}

/// A handle to either a compile-time static loot table or a datapack dynamic loot table.
#[derive(Clone, Debug)]
pub enum LootTableHandle {
    Static(&'static LootTable),
    Dynamic(std::sync::Arc<DynamicLootTable>),
}

impl From<&'static LootTable> for LootTableHandle {
    fn from(t: &'static LootTable) -> Self {
        Self::Static(t)
    }
}

impl From<std::sync::Arc<DynamicLootTable>> for LootTableHandle {
    fn from(t: std::sync::Arc<DynamicLootTable>) -> Self {
        Self::Dynamic(t)
    }
}

impl LootTableHandle {
    /// Vanilla `LootTable.randomSequence`.
    #[must_use]
    pub fn random_sequence(&self) -> Option<&str> {
        match self {
            Self::Static(table) => table.random_sequence,
            Self::Dynamic(table) => table.random_sequence.as_deref(),
        }
    }

    fn evaluate<R: RandomImpl>(
        &self,
        random: &mut R,
        params: &LootContextParameters<'_>,
    ) -> Vec<ItemStack> {
        match self {
            Self::Static(table) => evaluate_static(table, random, params),
            Self::Dynamic(table) => evaluate_dynamic(table, random, params),
        }
    }
}

#[must_use]
pub fn get_loot_table(key: &str) -> Option<LootTableHandle> {
    let full_key = if key.contains(':') {
        key.to_string()
    } else {
        format!("minecraft:{key}")
    };
    pumpkin_data::loot_table::get_loot_table(key)
        .or_else(|| pumpkin_data::loot_table::get_loot_table(&full_key))
        .map(LootTableHandle::Static)
}

/// Vanilla `LootContext.Builder.create`: a nonzero seed gets its own `LegacyRandomSource`
/// (`withOptionalRandomSeed`), else the table's random sequence, else the level's random. Runs
/// `$body` with `$random` bound to the chosen source, which stays locked until `$body` returns.
macro_rules! with_loot_random {
    ($handle:expr, $world:expr, $seed:expr, |$random:ident| $body:expr) => {{
        let handle: &LootTableHandle = $handle;
        let world: &World = $world;
        let seed: i64 = $seed;
        if seed != 0 {
            let $random = &mut LegacyRand::from_seed(seed as u64);
            $body
        } else if let Some((server, key)) = handle.random_sequence().and_then(|key| {
            let key = Identifier::parse(key).ok()?;
            Some((world.server.upgrade()?, key))
        }) {
            let world_seed = server.level_info.load().world_gen_settings.seed;
            let mut sequences = server
                .random_sequences
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let $random = sequences.get(&key, world_seed);
            $body
        } else {
            let mut level_random = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let $random = &mut *level_random;
            $body
        }
    }};
}

/// Vanilla `LootTable.getRandomItems(params, optionalLootTableSeed)`; a seed of 0 is
/// `getRandomItems(params)`.
#[must_use]
pub fn generate_loot_from_handle(
    handle: &LootTableHandle,
    world: &World,
    seed: i64,
    params: &LootContextParameters<'_>,
) -> Vec<ItemStack> {
    with_loot_random!(handle, world, seed, |random| handle
        .evaluate(random, params))
}

/// `inner`'s pools rolled on the random of `outer`'s context.
///
/// That is a `minecraft:loot_table` entry of `outer` picking `inner`, as vanilla's shearing
/// tables do by an entity-properties alternative Pumpkin's loot data can't express.
pub fn generate_nested_loot(
    outer: &LootTableHandle,
    inner: &LootTableHandle,
    world: &World,
    params: &LootContextParameters<'_>,
) -> Vec<ItemStack> {
    with_loot_random!(outer, world, 0, |random| inner.evaluate(random, params))
}

/// Vanilla `LootTable.fill`: the slot shuffle and stack splitting draw from the random the
/// items were rolled on.
pub fn fill_chest_inventory_handle(
    inventory: &std::sync::Arc<dyn pumpkin_inventory::Inventory>,
    handle: &LootTableHandle,
    world: &World,
    seed: i64,
    params: &LootContextParameters<'_>,
) {
    with_loot_random!(handle, world, seed, |random| fill(
        inventory, handle, random, params
    ));
}

/// Vanilla `RandomizableContainer.unpackLootTable`'s and
/// `ContainerEntity.unpackChestVehicleLootTable`'s parameters: `ORIGIN`, and with a player,
/// its luck and `THIS_ENTITY`.
#[must_use]
pub fn container_unpack_params(
    origin: pumpkin_util::math::vector3::Vector3<f64>,
    player: Option<&crate::entity::player::Player>,
) -> LootContextParameters<'_> {
    LootContextParameters {
        position: Some(origin),
        luck: player.map_or(0.0, |player| {
            player
                .living_entity
                .get_attribute_value(&pumpkin_data::attributes::Attributes::LUCK) as f32
        }),
        this_entity: player
            .map(|player| player.living_entity.entity.get_entity() as &dyn EntityBase),
        ..Default::default()
    }
}

fn fill<R: RandomImpl>(
    inventory: &std::sync::Arc<dyn pumpkin_inventory::Inventory>,
    handle: &LootTableHandle,
    random: &mut R,
    params: &LootContextParameters<'_>,
) {
    let mut items = handle.evaluate(random, params);
    let mut available_slots = get_available_slots(inventory, random);
    shuffle_and_split_items(&mut items, available_slots.len(), random);
    for item in items {
        let Some(slot) = available_slots.pop() else {
            tracing::warn!("Tried to over-fill a container");
            return;
        };
        inventory.set_stack(slot, item);
    }
}

/// Vanilla `LootTable.getAvailableSlots`.
fn get_available_slots<R: RandomImpl>(
    inventory: &std::sync::Arc<dyn pumpkin_inventory::Inventory>,
    random: &mut R,
) -> Vec<usize> {
    let mut slots: Vec<usize> = (0..inventory.size())
        .filter(|&slot| inventory.get_stack(slot).is_empty())
        .collect();
    shuffle(&mut slots, random);
    slots
}

/// Vanilla `LootTable.shuffleAndSplitItems`.
fn shuffle_and_split_items<R: RandomImpl>(
    result: &mut Vec<ItemStack>,
    available_slots: usize,
    random: &mut R,
) {
    let mut splittable: Vec<ItemStack> = Vec::new();
    let mut kept = Vec::with_capacity(result.len());
    for stack in result.drain(..) {
        if stack.is_empty() {
            continue;
        }
        if stack.item_count > 1 {
            splittable.push(stack);
        } else {
            kept.push(stack);
        }
    }
    *result = kept;

    while available_slots > result.len() + splittable.len() && !splittable.is_empty() {
        let index = random.next_inbetween_i32(0, splittable.len() as i32 - 1) as usize;
        let mut stack = splittable.remove(index);
        let remove = random.next_inbetween_i32(1, i32::from(stack.item_count) / 2);
        let copy = stack.split(remove as u8);

        if stack.item_count > 1 && random.next_bool() {
            splittable.push(stack);
        } else {
            result.push(stack);
        }
        if copy.item_count > 1 && random.next_bool() {
            splittable.push(copy);
        } else {
            result.push(copy);
        }
    }

    result.append(&mut splittable);
    shuffle(result, random);
}
