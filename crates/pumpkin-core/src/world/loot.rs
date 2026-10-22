use pumpkin_data::BlockState;
use pumpkin_data::Enchantment;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::EnchantmentsImpl;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::recipes::{CookingRecipeKind, get_cooking_recipe_with_ingredient};
use pumpkin_data::tag::Taggable;
use pumpkin_util::identifier::Identifier;
pub use pumpkin_util::loot_table::{
    DynamicLootCondition, DynamicLootEntry, DynamicLootPool, DynamicLootTable, LootBonusFormula,
    LootCondition, LootEntry, LootPool, LootTable,
};
use pumpkin_util::math::java_round_f32;
use pumpkin_util::random::legacy_rand::LegacyRand;
use pumpkin_util::random::{RandomImpl, shuffle};

use crate::world::World;

#[derive(Default, Clone)]
pub struct LootContextParameters {
    pub explosion_radius: Option<f32>,
    pub block_state: Option<&'static BlockState>,
    pub killed_by_player: Option<bool>,
    pub luck: f32,
    pub this_entity: Option<&'static EntityType>,
    pub killer_entity: Option<&'static EntityType>,
    /// Vanilla `EnchantmentHelper.getEnchantmentLevel(LOOTING, ATTACKING_ENTITY)`: 0 unless the
    /// attacker is a living entity with Looting in a slot Looting works from.
    pub attacking_entity_looting_level: i32,
    pub direct_killer_entity: Option<&'static EntityType>,
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
    pub damage_source_entity: Option<&'static EntityType>,
    /// The `minecraft:frog/variant` of that entity, when it is a frog.
    pub damage_source_frog_variant: Option<&'static str>,
    /// The size of the killed entity, when it is a slime or magma cube.
    pub this_cube_mob_size: Option<i32>,
}

/// Vanilla `DamageSourcePredicate.sourceEntity` on its type and frog variant.
fn damage_source_entity_matches(
    params: &LootContextParameters,
    entity_type: &str,
    frog_variant: Option<&str>,
) -> bool {
    let entity_type = entity_type
        .strip_prefix("minecraft:")
        .unwrap_or(entity_type);
    params
        .damage_source_entity
        .is_some_and(|source| source.resource_name == entity_type)
        && frog_variant.is_none_or(|variant| params.damage_source_frog_variant == Some(variant))
}

/// Vanilla `EntityTypePredicate` on `this`.
fn this_entity_type_matches(params: &LootContextParameters, entity_type: &str) -> bool {
    let entity_type = entity_type
        .strip_prefix("minecraft:")
        .unwrap_or(entity_type);
    params
        .this_entity
        .is_some_and(|this| this.resource_name == entity_type)
}

/// Vanilla `CubeMobPredicate`'s size range.
fn this_cube_mob_size_matches(params: &LootContextParameters, min: i32, max: i32) -> bool {
    params
        .this_cube_mob_size
        .is_some_and(|size| (min..=max).contains(&size))
}

fn direct_attacker_weapon_has_enchantment(params: &LootContextParameters, tag: &str) -> bool {
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

fn check_dynamic_condition<R: RandomImpl>(
    cond: &DynamicLootCondition,
    has_silk_touch: bool,
    has_shears: bool,
    fortune_level: i32,
    params: &LootContextParameters,
    rng: &mut R,
) -> bool {
    match cond {
        DynamicLootCondition::None => true,
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
            unenchanted_chance,
            enchanted_chance_base,
            enchanted_chance_per_level_above_first,
        } => {
            // Vanilla reads the attacker's Looting, not the tool.
            let level = params.attacking_entity_looting_level;
            let chance = if level > 0 {
                enchanted_chance_base + enchanted_chance_per_level_above_first * (level - 1) as f32
            } else {
                *unenchanted_chance
            };
            rng.next_f32() < chance
        }
        DynamicLootCondition::TableBonus { chances } => {
            let index = (fortune_level.max(0) as usize).min(chances.len().saturating_sub(1));
            chances
                .get(index)
                .is_some_and(|chance| rng.next_f32() < *chance)
        }
        DynamicLootCondition::AllOf(conditions) => conditions.iter().all(|c| {
            check_dynamic_condition(c, has_silk_touch, has_shears, fortune_level, params, rng)
        }),
        DynamicLootCondition::AnyOf(conditions) => conditions.iter().any(|c| {
            check_dynamic_condition(c, has_silk_touch, has_shears, fortune_level, params, rng)
        }),
        DynamicLootCondition::Inverted(cond) => {
            !check_dynamic_condition(cond, has_silk_touch, has_shears, fortune_level, params, rng)
        }
        DynamicLootCondition::EntityOnFire => params.is_on_fire.unwrap_or(false),
        DynamicLootCondition::DirectAttackerWeaponEnchantment(tag) => {
            direct_attacker_weapon_has_enchantment(params, tag)
        }
        DynamicLootCondition::DamageSourceEntity {
            entity_type,
            frog_variant,
        } => damage_source_entity_matches(params, entity_type, frog_variant.as_deref()),
        DynamicLootCondition::ThisCubeMobSize { min, max } => {
            this_cube_mob_size_matches(params, *min, *max)
        }
        DynamicLootCondition::ThisEntityType(entity_type) => {
            this_entity_type_matches(params, entity_type)
        }
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
    }
}

fn check_condition<R: RandomImpl>(
    cond: LootCondition,
    has_silk_touch: bool,
    has_shears: bool,
    fortune_level: i32,
    params: &LootContextParameters,
    rng: &mut R,
) -> bool {
    match cond {
        LootCondition::None => true,
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
            unenchanted_chance,
            enchanted_chance_base,
            enchanted_chance_per_level_above_first,
        } => {
            // Vanilla reads the attacker's Looting, not the tool.
            let level = params.attacking_entity_looting_level;
            let chance = if level > 0 {
                enchanted_chance_base + enchanted_chance_per_level_above_first * (level - 1) as f32
            } else {
                unenchanted_chance
            };
            rng.next_f32() < chance
        }
        LootCondition::TableBonus { chances } => {
            let index = (fortune_level.max(0) as usize).min(chances.len().saturating_sub(1));
            chances
                .get(index)
                .is_some_and(|chance| rng.next_f32() < *chance)
        }
        LootCondition::AllOf(conditions) => conditions
            .iter()
            .all(|c| check_condition(*c, has_silk_touch, has_shears, fortune_level, params, rng)),
        LootCondition::AnyOf(conditions) => conditions
            .iter()
            .any(|c| check_condition(*c, has_silk_touch, has_shears, fortune_level, params, rng)),
        LootCondition::EntityOnFire => params.is_on_fire.unwrap_or(false),
        LootCondition::DirectAttackerWeaponEnchantment(tag) => {
            direct_attacker_weapon_has_enchantment(params, tag)
        }
        LootCondition::DamageSourceEntity {
            entity_type,
            frog_variant,
        } => damage_source_entity_matches(params, entity_type, frog_variant),
        LootCondition::ThisCubeMobSize { min, max } => this_cube_mob_size_matches(params, min, max),
        LootCondition::ThisEntityType(entity_type) => this_entity_type_matches(params, entity_type),
        LootCondition::Inverted(cond) => !check_condition(
            *cond,
            has_silk_touch,
            has_shears,
            fortune_level,
            params,
            rng,
        ),
    }
}

/// The tool's Fortune level, which `apply_bonus` and `table_bonus` read.
fn tool_fortune_level(params: &LootContextParameters) -> i32 {
    params
        .tool
        .as_ref()
        .map_or(0, |tool| tool.get_enchantment_level(&Enchantment::FORTUNE))
}

fn tool_has_silk_touch(params: &LootContextParameters) -> bool {
    params
        .tool
        .as_ref()
        .is_some_and(|tool| tool.get_enchantment_level(&Enchantment::SILK_TOUCH) > 0)
}

fn tool_is_shears(params: &LootContextParameters) -> bool {
    params
        .tool
        .as_ref()
        .is_some_and(|tool| tool.item == &Item::SHEARS)
}

fn apply_bonus_formula<R: RandomImpl>(
    count: i32,
    bonus: LootBonusFormula,
    fortune_level: i32,
    params: &LootContextParameters,
    rng: &mut R,
) -> i32 {
    // Vanilla `ApplyBonusCount.run`: no `TOOL`, no formula. A block's loot always has one.
    if params.tool.is_none() && !matches!(bonus, LootBonusFormula::EnchantedCountIncrease { .. }) {
        return count;
    }
    match bonus {
        LootBonusFormula::OreDrops => {
            if fortune_level > 0 {
                let bonus = (rng.next_bounded_i32(fortune_level + 2) - 1).max(0);
                count * (bonus + 1)
            } else {
                count
            }
        }
        // Draws `nextInt(1)` even at level 0.
        LootBonusFormula::UniformBonusCount(bonus_multiplier) => {
            count + rng.next_bounded_i32(bonus_multiplier * fortune_level + 1)
        }
        LootBonusFormula::BinomialWithBonusCount { extra, probability } => {
            let n = fortune_level + extra;
            let mut bonus_count = 0;
            for _ in 0..n {
                if rng.next_f32() < probability {
                    bonus_count += 1;
                }
            }
            count + bonus_count
        }
        // Vanilla `EnchantedCountIncreaseFunction.run`.
        LootBonusFormula::EnchantedCountIncrease { min, max, limit } => {
            let level = params.attacking_entity_looting_level;
            if level == 0 {
                return count;
            }
            let count = count + java_round_f32(level as f32 * rng.mth_next_float(min, max));
            limit.map_or(count, |limit| count.min(limit))
        }
    }
}

/// Vanilla `LootPool.addRandomItem`'s pick among the entries that expanded with a positive
/// weight: a lone entry is taken without a draw.
fn pick_entry<'a, E, R: RandomImpl>(
    valid_entries: &[&'a E],
    weight: impl Fn(&E) -> i32,
    random: &mut R,
) -> Option<&'a E> {
    let total_weight: i32 = valid_entries.iter().map(|entry| weight(entry)).sum();
    if total_weight == 0 {
        return None;
    }
    if let [entry] = valid_entries {
        return Some(entry);
    }
    let mut index = random.next_bounded_i32(total_weight);
    valid_entries.iter().copied().find(|entry| {
        index -= weight(entry);
        index < 0
    })
}

/// Vanilla `LootTable.createStackSplitter`: `count` of `stack`'s item, in max-size pieces when
/// it reaches the max size.
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

/// The item an entry names and its count after `set_count` and the bonus count, before any
/// `furnace_smelt`.
fn roll_item_count<R: RandomImpl>(
    item: &str,
    min_count: i32,
    max_count: i32,
    bonus_formula: Option<LootBonusFormula>,
    fortune_level: i32,
    params: &LootContextParameters,
    random: &mut R,
) -> Option<(&'static Item, i32)> {
    // `set_count`: `Mth.nextInt`, no draw for a constant.
    let mut count = random.next_inbetween_i32(min_count, max_count);
    if let Some(bonus) = bonus_formula {
        count = apply_bonus_formula(count, bonus, fortune_level, params, random);
    }
    let item = Item::from_registry_key(item.strip_prefix("minecraft:").unwrap_or(item))?;
    Some((item, count))
}

fn evaluate_static<R: RandomImpl>(
    table: &LootTable,
    random: &mut R,
    params: &LootContextParameters,
) -> Vec<ItemStack> {
    let mut items_to_place: Vec<ItemStack> = Vec::new();
    let has_silk_touch = tool_has_silk_touch(params);
    let has_shears = tool_is_shears(params);
    let fortune_level = tool_fortune_level(params);
    let check = |cond: LootCondition, random: &mut R| {
        check_condition(
            cond,
            has_silk_touch,
            has_shears,
            fortune_level,
            params,
            random,
        )
    };

    // Vanilla `LootPool.addRandomItems`; no vanilla table has `bonus_rolls`, so luck adds
    // no rolls here.
    for pool in table.pools {
        if !check(pool.condition, random) {
            continue;
        }
        let rolls = random.next_inbetween_i32(pool.min_rolls, pool.max_rolls);
        for _ in 0..rolls {
            // Vanilla `LootPool.addRandomItem`: each roll expands the entries again, testing
            // their conditions.
            let mut valid_entries: Vec<&LootEntry> = Vec::new();
            for entry in pool.entries {
                if check(entry.condition, random) && entry.weight > 0 {
                    valid_entries.push(entry);
                }
            }
            let Some(entry) = pick_entry(&valid_entries, |entry| entry.weight, random) else {
                continue;
            };
            let Some(item) = entry.item else {
                continue;
            };
            let Some((item, count)) = roll_item_count(
                item,
                entry.min_count,
                entry.max_count,
                entry.bonus_formula,
                fortune_level,
                params,
                random,
            ) else {
                continue;
            };
            // The smelt condition is tested even on an empty stack; `SmeltItemFunction` then
            // skips it, and caps its result at the max size, so the cap below loses nothing.
            let smelts = entry.furnace_smelt.is_some_and(|c| check(c, random));
            if count <= 0 {
                continue;
            }
            if smelts {
                let smelted = smelt(ItemStack::new(u8::try_from(count).unwrap_or(u8::MAX), item));
                split_stack(&smelted, i32::from(smelted.item_count), &mut items_to_place);
            } else {
                split_stack(&ItemStack::new(1, item), count, &mut items_to_place);
            }
        }
    }

    items_to_place
}

fn evaluate_dynamic<R: RandomImpl>(
    table: &DynamicLootTable,
    random: &mut R,
    params: &LootContextParameters,
) -> Vec<ItemStack> {
    let mut items_to_place: Vec<ItemStack> = Vec::new();
    let has_silk_touch = tool_has_silk_touch(params);
    let has_shears = tool_is_shears(params);
    let fortune_level = tool_fortune_level(params);
    let check = |cond: &DynamicLootCondition, random: &mut R| {
        check_dynamic_condition(
            cond,
            has_silk_touch,
            has_shears,
            fortune_level,
            params,
            random,
        )
    };

    for pool in &table.pools {
        if !check(&pool.condition, random) {
            continue;
        }
        let rolls = random.next_inbetween_i32(pool.min_rolls, pool.max_rolls);
        for _ in 0..rolls {
            let mut valid_entries: Vec<&DynamicLootEntry> = Vec::new();
            for entry in &pool.entries {
                if check(&entry.condition, random) && entry.weight > 0 {
                    valid_entries.push(entry);
                }
            }
            let Some(entry) = pick_entry(&valid_entries, |entry| entry.weight, random) else {
                continue;
            };
            let Some(item) = &entry.item else {
                continue;
            };
            if let Some((item, count)) = roll_item_count(
                item,
                entry.min_count,
                entry.max_count,
                entry.bonus_formula,
                fortune_level,
                params,
                random,
            ) {
                split_stack(&ItemStack::new(1, item), count, &mut items_to_place);
            }
        }
    }

    items_to_place
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
        params: &LootContextParameters,
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
    params: &LootContextParameters,
) -> Vec<ItemStack> {
    with_loot_random!(handle, world, seed, |random| handle
        .evaluate(random, params))
}

/// Vanilla `LootTable.fill`: the slot shuffle and stack splitting draw from the random the
/// items were rolled on.
pub fn fill_chest_inventory_handle(
    inventory: &std::sync::Arc<dyn pumpkin_inventory::Inventory>,
    handle: &LootTableHandle,
    world: &World,
    seed: i64,
    params: &LootContextParameters,
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
) -> LootContextParameters {
    LootContextParameters {
        position: Some(origin),
        luck: player.map_or(0.0, |player| {
            player
                .living_entity
                .get_attribute_value(&pumpkin_data::attributes::Attributes::LUCK) as f32
        }),
        this_entity: player.map(|player| player.living_entity.entity.entity_type),
        ..Default::default()
    }
}

fn fill<R: RandomImpl>(
    inventory: &std::sync::Arc<dyn pumpkin_inventory::Inventory>,
    handle: &LootTableHandle,
    random: &mut R,
    params: &LootContextParameters,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pumpkin_data::entity::EntityType;
    use pumpkin_data::item::Item;
    use pumpkin_data::item_stack::ItemStack;
    use pumpkin_data::loot_table::{BLOCKS_MELON, ENTITIES_MAGMA_CUBE, ENTITIES_SKELETON};
    use pumpkin_inventory::{Inventory, SimpleInventory};
    use pumpkin_util::loot_table::{LootCondition, LootEntry, LootPool, LootTable};
    use pumpkin_util::random::RandomImpl;
    use pumpkin_util::random::legacy_rand::LegacyRand;
    use pumpkin_util::random::trace::{Draw, TraceRandom};

    use super::{LootContextParameters, LootTableHandle, evaluate_static, fill};

    fn magma_cube_drops(
        killer: &'static EntityType,
        frog_variant: Option<&'static str>,
    ) -> Vec<&'static Item> {
        (0..64)
            .flat_map(|seed| {
                evaluate_static(
                    &ENTITIES_MAGMA_CUBE,
                    &mut LegacyRand::from_seed(seed),
                    &LootContextParameters {
                        this_entity: Some(&EntityType::MAGMA_CUBE),
                        damage_source_entity: Some(killer),
                        damage_source_frog_variant: frog_variant,
                        this_cube_mob_size: Some(1),
                        ..Default::default()
                    },
                )
            })
            .map(|stack| stack.item)
            .collect()
    }

    #[test]
    fn a_frog_eating_a_magma_cube_drops_its_variants_froglight() {
        let drops = magma_cube_drops(&EntityType::FROG, Some("minecraft:warm"));
        assert_eq!(drops.len(), 64);
        assert!(
            drops
                .iter()
                .all(|item| *item == &Item::PEARLESCENT_FROGLIGHT)
        );
    }

    #[test]
    fn a_tiny_magma_cube_killed_by_anything_else_drops_nothing() {
        assert!(magma_cube_drops(&EntityType::ZOMBIE, None).is_empty());
    }

    /// `LootPool.addRandomItem` skips the pick for a single entry, and
    /// `EnchantedCountIncreaseFunction` skips its draw without a looting killer, so only each
    /// pool's `set_count` draws.
    #[test]
    fn a_skeleton_without_a_killer_draws_only_its_two_counts() {
        let mut random = TraceRandom::from_seed(0);
        evaluate_static(
            &ENTITIES_SKELETON,
            &mut random,
            &LootContextParameters {
                this_entity: Some(&EntityType::SKELETON),
                ..Default::default()
            },
        );
        assert_eq!(
            random.draws,
            [Draw::NextIntBounded(3), Draw::NextIntBounded(3)]
        );
    }

    /// `EnchantedCountIncreaseFunction` reads the killer's Looting and draws a float count.
    #[test]
    fn a_skeleton_killed_with_looting_draws_a_float_per_count() {
        let mut random = TraceRandom::from_seed(0);
        evaluate_static(
            &ENTITIES_SKELETON,
            &mut random,
            &LootContextParameters {
                this_entity: Some(&EntityType::SKELETON),
                attacking_entity_looting_level: 2,
                ..Default::default()
            },
        );
        assert_eq!(
            random.draws,
            [
                Draw::NextIntBounded(3),
                Draw::NextFloat,
                Draw::NextIntBounded(3),
                Draw::NextFloat,
            ]
        );
    }

    /// `ApplyBonusCount.UniformBonusCount` draws `nextInt(multiplier * level + 1)` even without
    /// Fortune, as long as there is a `TOOL` (an empty hand is `ItemStack.EMPTY`).
    #[test]
    fn a_melon_broken_by_hand_draws_its_uniform_bonus() {
        let mut random = TraceRandom::from_seed(0);
        evaluate_static(
            &BLOCKS_MELON,
            &mut random,
            &LootContextParameters {
                tool: Some(ItemStack::EMPTY.clone()),
                ..Default::default()
            },
        );
        assert_eq!(
            random.draws,
            [Draw::NextIntBounded(5), Draw::NextIntBounded(1)]
        );
    }

    /// Vanilla walks the entries in order, so an `empty` entry listed after an item takes the
    /// upper part of the pick.
    #[test]
    fn an_empty_entry_keeps_its_place_in_the_pick() {
        static DIAMOND_THEN_EMPTY: LootTable = LootTable {
            random_sequence: None,
            pools: &[LootPool {
                entries: &[
                    LootEntry {
                        item: Some("minecraft:diamond"),
                        weight: 1,
                        min_count: 1,
                        max_count: 1,
                        condition: LootCondition::None,
                        bonus_formula: None,
                        furnace_smelt: None,
                    },
                    LootEntry {
                        item: None,
                        weight: 1,
                        min_count: 1,
                        max_count: 1,
                        condition: LootCondition::None,
                        bonus_formula: None,
                        furnace_smelt: None,
                    },
                ],
                min_rolls: 1,
                max_rolls: 1,
                condition: LootCondition::None,
            }],
        };
        for seed in 0..32 {
            let drops = evaluate_static(
                &DIAMOND_THEN_EMPTY,
                &mut LegacyRand::from_seed(seed),
                &LootContextParameters::default(),
            );
            let picks_diamond = LegacyRand::from_seed(seed).next_bounded_i32(2) == 0;
            assert_eq!(drops.len(), usize::from(picks_diamond), "seed {seed}");
        }
    }

    static EMPTY_TABLE: LootTable = LootTable {
        random_sequence: None,
        pools: &[],
    };

    static TWO_DIAMONDS: LootTable = LootTable {
        random_sequence: None,
        pools: &[LootPool {
            entries: &[LootEntry {
                item: Some("minecraft:diamond"),
                weight: 1,
                min_count: 2,
                max_count: 2,
                condition: LootCondition::None,
                bonus_formula: None,
                furnace_smelt: None,
            }],
            min_rolls: 1,
            max_rolls: 1,
            condition: LootCondition::None,
        }],
    };

    fn fill_draws(table: &'static LootTable, slots: usize) -> Vec<Draw> {
        let inventory: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(slots));
        let mut random = TraceRandom::from_seed(0);
        fill(
            &inventory,
            &LootTableHandle::Static(table),
            &mut random,
            &LootContextParameters::default(),
        );
        random.draws
    }

    /// Vanilla `LootTable.fill` shuffles the empty slots even when nothing rolled.
    #[test]
    fn fill_shuffles_the_slots_when_the_table_rolls_nothing() {
        assert_eq!(
            fill_draws(&EMPTY_TABLE, 3),
            [Draw::NextIntBounded(3), Draw::NextIntBounded(2)]
        );
    }

    /// `shuffleAndSplitItems` picks and splits with `Mth.nextInt`, which skips the draw for a
    /// single splittable stack and a split of exactly one.
    #[test]
    fn fill_splits_a_pair_without_drawing() {
        assert_eq!(
            fill_draws(&TWO_DIAMONDS, 3),
            [
                Draw::NextIntBounded(3),
                Draw::NextIntBounded(2),
                Draw::NextIntBounded(2),
            ]
        );
    }
}
