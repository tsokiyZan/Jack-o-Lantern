use crate::entity::EntityBase;
use crate::world::World;
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
use pumpkin_util::random::legacy_rand::LegacyRand;
use pumpkin_util::random::{RandomImpl, shuffle};

mod generation;
mod predicates;
#[cfg(test)]
mod tests;

/// Vanilla `LootParams`. The entities are the live ones, so predicates read their current state.
#[derive(Default, Clone)]
pub struct LootContextParameters<'a> {
    pub world: Option<&'a World>,
    pub explosion_radius: Option<f32>,
    pub block_state: Option<&'a BlockState>,
    pub killed_by_player: Option<bool>,
    pub luck: f32,
    /// `THIS_ENTITY`.
    pub this_entity: Option<&'a dyn EntityBase>,
    /// `ATTACKING_ENTITY`: `DamageSource.getEntity`.
    pub killer_entity: Option<&'a dyn EntityBase>,
    /// Vanilla `EnchantmentHelper.getEnchantmentLevel(LOOTING, ATTACKING_ENTITY)`: 0 unless the
    /// attacker is a living entity with Looting in a slot Looting works from.
    pub attacking_entity_looting_level: i32,
    /// `DIRECT_ATTACKING_ENTITY`: `DamageSource.getDirectEntity`.
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
}

/// Vanilla `EnchantmentHelper.getEnchantmentLevel(enchantment, ATTACKING_ENTITY)`: read from
/// the attacker's equipment, 0 for a missing or non-living attacker.
///
/// A caller that has no attacker entity but a Looting level (`attacking_entity_looting_level`)
/// still gets it for Looting.
fn attacker_enchantment_level(params: &LootContextParameters<'_>, name: &str) -> i32 {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let Some(killer) = params.killer_entity else {
        return if name == "looting" {
            params.attacking_entity_looting_level
        } else {
            0
        };
    };
    Enchantment::from_name(name)
        .and_then(|enchantment| {
            let living = killer.get_living_entity()?;
            Some(
                crate::enchantment::EnchantmentHelper::get_enchantment_level(
                    enchantment,
                    killer,
                    living,
                ),
            )
        })
        .unwrap_or(0)
}

/// Vanilla `EnchantmentHelper.getItemEnchantmentLevel(enchantment, TOOL)`.
fn tool_enchantment_level(params: &LootContextParameters<'_>, name: &str) -> i32 {
    Enchantment::from_name(name.strip_prefix("minecraft:").unwrap_or(name))
        .and_then(|enchantment| {
            params
                .tool
                .as_ref()
                .filter(|tool| !tool.is_empty())
                .map(|tool| tool.get_enchantment_level(enchantment))
        })
        .unwrap_or(0)
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

/// Vanilla `SmeltItemFunction.run` on `count` of `item`: the recipe result's count times the
/// input count, capped at the result's max stack size.
fn smelt(item: &'static Item, count: i32) -> (&'static Item, i32) {
    let result =
        get_cooking_recipe_with_ingredient(item, CookingRecipeKind::Smelting).and_then(|recipe| {
            let id = recipe
                .result
                .id
                .strip_prefix("minecraft:")
                .unwrap_or(recipe.result.id);
            Item::from_registry_key(id).map(|item| (item, recipe.result.count))
        });
    let Some((result, multiplier)) = result else {
        tracing::warn!(
            "Couldn't smelt {} because there is no smelting recipe",
            item.registry_key
        );
        return (item, count);
    };
    let max = i32::from(ItemStack::new(1, result).get_max_stack_size());
    (result, count.saturating_mul(i32::from(multiplier)).min(max))
}

fn weather_matches(
    params: &LootContextParameters<'_>,
    raining: Option<bool>,
    thundering: Option<bool>,
) -> bool {
    raining.is_none_or(|value| {
        params
            .is_raining
            .or_else(|| params.world.map(World::is_raining))
            .unwrap_or(false)
            == value
    }) && thundering.is_none_or(|value| {
        params
            .is_thundering
            .or_else(|| params.world.map(World::is_thundering))
            .unwrap_or(false)
            == value
    })
}

fn entity_on_fire(params: &LootContextParameters<'_>) -> bool {
    params.is_on_fire.unwrap_or_else(|| {
        params
            .this_entity
            .is_some_and(|entity| entity.get_entity().is_on_fire())
    })
}

fn check_dynamic_condition<R: RandomImpl>(
    cond: &DynamicLootCondition,
    has_silk_touch: bool,
    has_shears: bool,
    params: &LootContextParameters<'_>,
    rng: &mut R,
) -> bool {
    match cond {
        DynamicLootCondition::None => true,
        DynamicLootCondition::Never => false,
        DynamicLootCondition::LocationCheck { offset, predicate } => {
            predicates::matches_location(*offset, predicate, params)
        }
        DynamicLootCondition::MatchBlock(predicate) => predicates::matches_block(predicate, params),
        DynamicLootCondition::MatchTool(predicate) => predicates::matches_tool(predicate, params),
        DynamicLootCondition::WeatherCheck {
            raining,
            thundering,
        } => weather_matches(params, *raining, *thundering),
        DynamicLootCondition::EntityProperties { target, predicate } => {
            predicates::matches_entity(*target, predicate, params)
        }
        DynamicLootCondition::DamageSourceTag { tag, expected } => params
            .damage_type
            .is_some_and(|damage| damage.is_tagged_with(tag) == Some(*expected)),
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
            enchantment,
            unenchanted_chance,
            enchanted_chance_base,
            enchanted_chance_per_level_above_first,
        } => {
            let level = attacker_enchantment_level(params, enchantment);
            let chance = if level > 0 {
                enchanted_chance_base + enchanted_chance_per_level_above_first * (level - 1) as f32
            } else {
                *unenchanted_chance
            };
            rng.next_f32() < chance
        }
        DynamicLootCondition::TableBonus {
            enchantment,
            chances,
        } => {
            let index = (tool_enchantment_level(params, enchantment).max(0) as usize)
                .min(chances.len().saturating_sub(1));
            chances
                .get(index)
                .is_some_and(|chance| rng.next_f32() < *chance)
        }
        DynamicLootCondition::AllOf(conditions) => conditions
            .iter()
            .all(|c| check_dynamic_condition(c, has_silk_touch, has_shears, params, rng)),
        DynamicLootCondition::AnyOf(conditions) => conditions
            .iter()
            .any(|c| check_dynamic_condition(c, has_silk_touch, has_shears, params, rng)),
        DynamicLootCondition::Inverted(cond) => {
            !check_dynamic_condition(cond, has_silk_touch, has_shears, params, rng)
        }
        DynamicLootCondition::EntityOnFire => entity_on_fire(params),
        DynamicLootCondition::DirectAttackerWeaponEnchantment(tag) => {
            direct_attacker_weapon_has_enchantment(params, tag)
        }
    }
}

fn check_condition<R: RandomImpl>(
    cond: LootCondition,
    has_silk_touch: bool,
    has_shears: bool,
    params: &LootContextParameters<'_>,
    rng: &mut R,
) -> bool {
    match cond {
        LootCondition::None => true,
        LootCondition::Never => false,
        LootCondition::LocationCheck { offset, predicate } => {
            predicates::matches_location(offset, predicate, params)
        }
        LootCondition::MatchBlock(predicate) => predicates::matches_block(predicate, params),
        LootCondition::MatchTool(predicate) => predicates::matches_tool(predicate, params),
        LootCondition::WeatherCheck {
            raining,
            thundering,
        } => weather_matches(params, raining, thundering),
        LootCondition::EntityProperties { target, predicate } => {
            predicates::matches_entity(target, predicate, params)
        }
        LootCondition::DamageSourceTag { tag, expected } => params
            .damage_type
            .is_some_and(|damage| damage.is_tagged_with(tag) == Some(expected)),
        LootCondition::Inverted(condition) => {
            !check_condition(*condition, has_silk_touch, has_shears, params, rng)
        }
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
            enchantment,
            unenchanted_chance,
            enchanted_chance_base,
            enchanted_chance_per_level_above_first,
        } => {
            let level = attacker_enchantment_level(params, enchantment);
            let chance = if level > 0 {
                enchanted_chance_base + enchanted_chance_per_level_above_first * (level - 1) as f32
            } else {
                unenchanted_chance
            };
            rng.next_f32() < chance
        }
        LootCondition::TableBonus {
            enchantment,
            chances,
        } => {
            let index = (tool_enchantment_level(params, enchantment).max(0) as usize)
                .min(chances.len().saturating_sub(1));
            chances
                .get(index)
                .is_some_and(|chance| rng.next_f32() < *chance)
        }
        LootCondition::AllOf(conditions) => conditions
            .iter()
            .all(|c| check_condition(*c, has_silk_touch, has_shears, params, rng)),
        LootCondition::AnyOf(conditions) => conditions
            .iter()
            .any(|c| check_condition(*c, has_silk_touch, has_shears, params, rng)),
        LootCondition::EntityOnFire => entity_on_fire(params),
        LootCondition::DirectAttackerWeaponEnchantment(tag) => {
            direct_attacker_weapon_has_enchantment(params, tag)
        }
    }
}

/// `ApplyBonusCount`'s formulas.
fn apply_bonus_formula<R: RandomImpl>(
    count: i32,
    bonus: LootBonusFormula,
    level: i32,
    rng: &mut R,
) -> i32 {
    match bonus {
        LootBonusFormula::OreDrops => {
            if level > 0 {
                let bonus = (rng.next_bounded_i32(level + 2) - 1).max(0);
                count * (bonus + 1)
            } else {
                count
            }
        }
        // Draws `nextInt(1)` even at level 0.
        LootBonusFormula::UniformBonusCount(bonus_multiplier) => {
            let bound = level.saturating_mul(bonus_multiplier).saturating_add(1);
            if bound > 0 {
                count.saturating_add(rng.next_bounded_i32(bound))
            } else {
                count
            }
        }
        LootBonusFormula::BinomialWithBonusCount { extra, probability } => {
            let mut count = count;
            for _ in 0..level + extra {
                if rng.next_f32() < probability {
                    count += 1;
                }
            }
            count
        }
    }
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
fn pick_entry<'a, E, R: RandomImpl>(
    valid_entries: &[&'a E],
    weight: impl Fn(&E) -> i32,
    random: &mut R,
) -> Option<&'a E> {
    let total_weight = valid_entries
        .iter()
        .try_fold(0i32, |sum, entry| sum.checked_add(weight(entry)))?;
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
            Self::Static(table) => {
                generation::generate(generation::Table::Static(table), random, params)
            }
            Self::Dynamic(table) => {
                generation::generate(generation::Table::Dynamic(table), random, params)
            }
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

/// `params` with the level the table rolls in, which `LootParams` always carries.
fn with_world<'a>(
    params: &LootContextParameters<'a>,
    world: &'a World,
) -> LootContextParameters<'a> {
    LootContextParameters {
        world: params.world.or(Some(world)),
        ..params.clone()
    }
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
    let params = with_world(params, world);
    with_loot_random!(handle, world, seed, |random| handle
        .evaluate(random, &params))
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
    let params = with_world(params, world);
    with_loot_random!(outer, world, 0, |random| inner.evaluate(random, &params))
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
    let params = with_world(params, world);
    with_loot_random!(handle, world, seed, |random| fill(
        inventory, handle, random, &params
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
        this_entity: player.map(|player| player as &dyn EntityBase),
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

#[cfg(test)]
mod draw_order_tests;
