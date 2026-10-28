//! The draws a table makes on its random source, in vanilla's order. These run on a
//! `TraceRandom`, so they check the sequence of calls and not only the items.

use std::sync::Arc;

use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::loot_table::{BLOCKS_MELON, ENTITIES_MAGMA_CUBE, ENTITIES_SKELETON};
use pumpkin_inventory::{Inventory, SimpleInventory};
use pumpkin_util::loot_table::{
    LootCondition, LootEntry, LootEntryKind, LootNumberProvider, LootPool, LootTable,
};
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_util::random::legacy_rand::LegacyRand;
use pumpkin_util::random::trace::{Draw, TraceRandom};

use super::{LootContextParameters, LootTableHandle, fill};
use crate::entity::{
    Entity,
    mob::cube_mob::{CubeKind, CubeMobEntity},
    passive::{chicken::ChickenEntity, frog::FrogEntity},
};

fn roll<R: RandomImpl>(
    table: &'static LootTable,
    random: &mut R,
    params: &LootContextParameters<'_>,
) -> Vec<ItemStack> {
    LootTableHandle::Static(table).evaluate(random, params)
}

const fn constant_pool(entries: &'static [LootEntry]) -> LootPool {
    LootPool {
        entries,
        rolls: LootNumberProvider::Constant(1.0),
        bonus_rolls: LootNumberProvider::Constant(0.0),
        condition: LootCondition::None,
        functions: &[],
    }
}

const fn entry(kind: LootEntryKind) -> LootEntry {
    LootEntry {
        kind,
        weight: 1,
        quality: 0,
        condition: LootCondition::None,
        functions: &[],
    }
}

/// A tiny magma cube dies to `killer`; every seed rolls the table once.
async fn magma_cube_drops(killer: &dyn crate::entity::EntityBase, size: i32) -> Vec<&'static Item> {
    let (_folder, world) = crate::world::test_support::test_world();
    let cube = CubeMobEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::MAGMA_CUBE),
        CubeKind::MagmaCube,
    );
    cube.set_size(size, true);
    let drops = (0..64)
        .flat_map(|seed| {
            roll(
                &ENTITIES_MAGMA_CUBE,
                &mut LegacyRand::from_seed(seed),
                &LootContextParameters {
                    world: Some(&world),
                    this_entity: Some(cube.as_ref()),
                    killer_entity: Some(killer),
                    ..Default::default()
                },
            )
        })
        .map(|stack| stack.item)
        .collect();
    world.level.shutdown().await;
    drops
}

#[tokio::test]
async fn a_frog_eating_a_magma_cube_drops_its_variants_froglight() {
    let (_folder, world) = crate::world::test_support::test_world();
    let frog = FrogEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::FROG,
    ));
    frog.set_variant(pumpkin_data::frog_variant::FrogVariant::Warm);
    let drops = magma_cube_drops(frog.as_ref(), 1).await;
    assert_eq!(drops.len(), 64);
    assert!(
        drops
            .iter()
            .all(|item| *item == &Item::PEARLESCENT_FROGLIGHT)
    );
    world.level.shutdown().await;
}

#[tokio::test]
async fn a_tiny_magma_cube_killed_by_anything_else_drops_nothing() {
    let (_folder, world) = crate::world::test_support::test_world();
    let chicken = ChickenEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::CHICKEN,
    ));
    assert!(magma_cube_drops(chicken.as_ref(), 1).await.is_empty());
    world.level.shutdown().await;
}

/// `LootPool.addRandomItem` skips the pick for a single entry, and
/// `EnchantedCountIncreaseFunction` skips its draw without a looting killer, so only each
/// pool's `set_count` draws.
#[test]
fn a_skeleton_without_a_killer_draws_only_its_two_counts() {
    let mut random = TraceRandom::from_seed(0);
    roll(
        &ENTITIES_SKELETON,
        &mut random,
        &LootContextParameters::default(),
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
    roll(
        &ENTITIES_SKELETON,
        &mut random,
        &LootContextParameters {
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
    roll(
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
        pools: &[constant_pool(&[
            entry(LootEntryKind::Item("minecraft:diamond")),
            entry(LootEntryKind::Empty),
        ])],
        functions: &[],
    };
    for seed in 0..32 {
        let drops = roll(
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
    functions: &[],
};

static TWO_DIAMONDS: LootTable = LootTable {
    random_sequence: None,
    pools: &[constant_pool(&[LootEntry {
        functions: &[pumpkin_util::loot_table::LootFunction {
            kind: pumpkin_util::loot_table::LootFunctionKind::SetCount {
                count: LootNumberProvider::Constant(2.0),
                add: false,
            },
            enchantment: None,
            condition: LootCondition::None,
            json: None,
        }],
        ..entry(LootEntryKind::Item("minecraft:diamond"))
    }])],
    functions: &[],
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
