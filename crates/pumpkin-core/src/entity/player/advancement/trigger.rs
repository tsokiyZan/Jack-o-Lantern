use std::sync::Arc;

use crate::entity::EntityBase;
use crate::entity::ageable::AgeableMob;
use crate::entity::mob::cube_mob::CubeMobEntity;
use crate::entity::mob::piglin::PiglinEntity;
use crate::entity::player::Player;
use pumpkin_data::Block;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::vector3::Vector3;

#[derive(Clone)]
pub enum AdvancementTrigger {
    InventoryChanged,
    PlayerKilledEntity {
        entity_type_resource: String,
    },
    /// Vanilla `CriteriaTriggers.ENTER_BLOCK`: the player is inside `block`.
    EnterBlock {
        block: &'static Block,
    },
    PlacedBlock {
        block_id: String,
    },
    ConsumeItem {
        item_id: String,
    },
    SleptInBed,
    FishedItem {
        item_id: String,
    },
    EnterDimension {
        dimension: String,
    },
    PlayerKilled,
    DeflectedDamage,
    LaunchedEyeOfEnder,
    GlowedSign,
    BredAnimal {
        parent_type: String,
    },
    DealtOverkillDamage,
    SniperDuel,
    TwoBirdsOneArrow,
    Arbalistic,
    Bullseye,
    CuredZombieVillager,
    TradedWithVillager,
    /// Vanilla `CriteriaTriggers.FALL_FROM_HEIGHT`: a fall that started at `start` ended.
    FallFromHeight {
        start: Vector3<f64>,
    },
    /// Vanilla `CriteriaTriggers.FALL_AFTER_EXPLOSION`: a fall started after an explosion
    /// at `start`, caused by an entity of type `cause`.
    FallAfterExplosion {
        start: Vector3<f64>,
        cause: Option<&'static EntityType>,
    },
    /// Vanilla `CriteriaTriggers.AVOID_VIBRATION`.
    AvoidVibration,
    /// Vanilla `CriteriaTriggers.USED_TOTEM`: `item` kept the player alive.
    UsedTotem {
        item: &'static Item,
    },
    /// Vanilla `CriteriaTriggers.THROWN_ITEM_PICKED_UP_BY_ENTITY`: `entity` picked up `item`,
    /// which this player threw.
    ThrownItemPickedUpByEntity {
        item: ItemStack,
        entity: Arc<dyn EntityBase>,
    },
    /// Vanilla `CriteriaTriggers.THROWN_ITEM_PICKED_UP_BY_PLAYER`: this player picked up `item`,
    /// which `thrower` threw.
    ThrownItemPickedUpByPlayer {
        item: ItemStack,
        thrower: Arc<dyn EntityBase>,
    },
}

/// Vanilla `EntityFlagsPredicate.isBaby(false)` on a piglin or a sulfur cube.
fn is_adult(entity: &dyn EntityBase) -> bool {
    let any = entity.cast_any();
    if let Some(piglin) = any.downcast_ref::<PiglinEntity>() {
        return !piglin.is_baby();
    }
    any.downcast_ref::<CubeMobEntity>()
        .is_some_and(|cube| !cube.is_baby())
}

impl Player {
    #[allow(clippy::collapsible_if, clippy::too_many_lines)]
    pub fn trigger_advancement(&self, trigger: AdvancementTrigger) {
        use pumpkin_data::advancement::Advancement;

        match trigger {
            AdvancementTrigger::InventoryChanged => {
                if !self.has_advancement(Advancement::STORY_ROOT) {
                    if self.has_item_in_inventory(&Item::CRAFTING_TABLE) {
                        self.trigger_advancement_criterion(
                            Advancement::STORY_ROOT,
                            "crafting_table",
                        );
                    }
                }

                if !self.has_advancement(Advancement::STORY_MINE_STONE) {
                    let stone_items = [
                        &Item::COBBLESTONE,
                        &Item::STONE,
                        &Item::ANDESITE,
                        &Item::DIORITE,
                        &Item::GRANITE,
                        &Item::DEEPSLATE,
                        &Item::COBBLED_DEEPSLATE,
                        &Item::BLACKSTONE,
                    ];
                    for item in stone_items {
                        if self.has_item_in_inventory(item) {
                            self.trigger_advancement_criterion(
                                Advancement::STORY_MINE_STONE,
                                "get_stone",
                            );
                            break;
                        }
                    }
                }

                if !self.has_advancement(Advancement::STORY_UPGRADE_TOOLS) {
                    if self.has_item_in_inventory(&Item::STONE_PICKAXE) {
                        self.trigger_advancement_criterion(
                            Advancement::STORY_UPGRADE_TOOLS,
                            "stone_pickaxe",
                        );
                    }
                }

                if !self.has_advancement(Advancement::STORY_SMELT_IRON) {
                    if self.has_item_in_inventory(&Item::IRON_INGOT) {
                        self.trigger_advancement_criterion(Advancement::STORY_SMELT_IRON, "iron");
                    }
                }

                if !self.has_advancement(Advancement::STORY_IRON_TOOLS) {
                    if self.has_item_in_inventory(&Item::IRON_PICKAXE) {
                        self.trigger_advancement_criterion(
                            Advancement::STORY_IRON_TOOLS,
                            "iron_pickaxe",
                        );
                    }
                }

                if !self.has_advancement(Advancement::STORY_MINE_DIAMOND) {
                    if self.has_item_in_inventory(&Item::DIAMOND) {
                        self.trigger_advancement_criterion(
                            Advancement::STORY_MINE_DIAMOND,
                            "diamond",
                        );
                    }
                }

                if !self.has_advancement(Advancement::STORY_LAVA_BUCKET) {
                    if self.has_item_in_inventory(&Item::LAVA_BUCKET) {
                        self.trigger_advancement_criterion(
                            Advancement::STORY_LAVA_BUCKET,
                            "lava_bucket",
                        );
                    }
                }

                if !self.has_advancement(Advancement::STORY_OBTAIN_ARMOR) {
                    let armor = [
                        (&Item::IRON_HELMET, "iron_helmet"),
                        (&Item::IRON_CHESTPLATE, "iron_chestplate"),
                        (&Item::IRON_LEGGINGS, "iron_leggings"),
                        (&Item::IRON_BOOTS, "iron_boots"),
                    ];
                    for (item, criterion) in armor {
                        if self.has_item_in_inventory(item) {
                            self.trigger_advancement_criterion(
                                Advancement::STORY_OBTAIN_ARMOR,
                                criterion,
                            );
                        }
                    }
                }

                if !self.has_advancement(Advancement::STORY_SHINY_GEAR) {
                    let armor = [
                        (&Item::DIAMOND_HELMET, "diamond_helmet"),
                        (&Item::DIAMOND_CHESTPLATE, "diamond_chestplate"),
                        (&Item::DIAMOND_LEGGINGS, "diamond_leggings"),
                        (&Item::DIAMOND_BOOTS, "diamond_boots"),
                    ];
                    for (item, criterion) in armor {
                        if self.has_item_in_inventory(item) {
                            self.trigger_advancement_criterion(
                                Advancement::STORY_SHINY_GEAR,
                                criterion,
                            );
                        }
                    }
                }

                if !self.has_advancement(Advancement::STORY_FORM_OBSIDIAN) {
                    if self.has_item_in_inventory(&Item::OBSIDIAN) {
                        self.trigger_advancement_criterion(
                            Advancement::STORY_FORM_OBSIDIAN,
                            "obsidian",
                        );
                    }
                }

                if !self.has_advancement(Advancement::NETHER_GET_WITHER_SKULL) {
                    if self.has_item_in_inventory(&Item::WITHER_SKELETON_SKULL) {
                        self.trigger_advancement_criterion(
                            Advancement::NETHER_GET_WITHER_SKULL,
                            "wither_skull",
                        );
                    }
                }

                if !self.has_advancement(Advancement::NETHER_OBTAIN_ANCIENT_DEBRIS) {
                    if self.has_item_in_inventory(&Item::ANCIENT_DEBRIS) {
                        self.trigger_advancement_criterion(
                            Advancement::NETHER_OBTAIN_ANCIENT_DEBRIS,
                            "ancient_debris",
                        );
                    }
                }

                if !self.has_advancement(Advancement::NETHER_OBTAIN_BLAZE_ROD) {
                    if self.has_item_in_inventory(&Item::BLAZE_ROD) {
                        self.trigger_advancement_criterion(
                            Advancement::NETHER_OBTAIN_BLAZE_ROD,
                            "blaze_rod",
                        );
                    }
                }

                if !self.has_advancement(Advancement::NETHER_OBTAIN_CRYING_OBSIDIAN) {
                    if self.has_item_in_inventory(&Item::CRYING_OBSIDIAN) {
                        self.trigger_advancement_criterion(
                            Advancement::NETHER_OBTAIN_CRYING_OBSIDIAN,
                            "crying_obsidian",
                        );
                    }
                }

                if !self.has_advancement(Advancement::NETHER_NETHERITE_ARMOR) {
                    if self.has_item_in_inventory(&Item::NETHERITE_HELMET)
                        && self.has_item_in_inventory(&Item::NETHERITE_CHESTPLATE)
                        && self.has_item_in_inventory(&Item::NETHERITE_LEGGINGS)
                        && self.has_item_in_inventory(&Item::NETHERITE_BOOTS)
                    {
                        self.trigger_advancement_criterion(
                            Advancement::NETHER_NETHERITE_ARMOR,
                            "netherite_armor",
                        );
                    }
                }

                if !self.has_advancement(Advancement::HUSBANDRY_TACTICAL_FISHING) {
                    let fish_buckets = [
                        (&Item::COD_BUCKET, "cod_bucket"),
                        (&Item::SALMON_BUCKET, "salmon_bucket"),
                        (&Item::PUFFERFISH_BUCKET, "pufferfish_bucket"),
                        (&Item::TROPICAL_FISH_BUCKET, "tropical_fish_bucket"),
                    ];
                    for (item, criterion) in fish_buckets {
                        if self.has_item_in_inventory(item) {
                            self.trigger_advancement_criterion(
                                Advancement::HUSBANDRY_TACTICAL_FISHING,
                                criterion,
                            );
                        }
                    }
                }

                if !self.has_advancement(Advancement::HUSBANDRY_AXOLOTL_IN_A_BUCKET) {
                    if self.has_item_in_inventory(&Item::AXOLOTL_BUCKET) {
                        self.trigger_advancement_criterion(
                            Advancement::HUSBANDRY_AXOLOTL_IN_A_BUCKET,
                            "axolotl_bucket",
                        );
                    }
                }

                if !self.has_advancement(Advancement::HUSBANDRY_TADPOLE_IN_A_BUCKET) {
                    if self.has_item_in_inventory(&Item::TADPOLE_BUCKET) {
                        self.trigger_advancement_criterion(
                            Advancement::HUSBANDRY_TADPOLE_IN_A_BUCKET,
                            "tadpole_bucket",
                        );
                    }
                }

                if !self.has_advancement(Advancement::HUSBANDRY_OBTAIN_NETHERITE_HOE) {
                    if self.has_item_in_inventory(&Item::NETHERITE_HOE) {
                        self.trigger_advancement_criterion(
                            Advancement::HUSBANDRY_OBTAIN_NETHERITE_HOE,
                            "netherite_hoe",
                        );
                    }
                }

                if !self.has_advancement(Advancement::STORY_ENCHANT_ITEM) {
                    let has_enchanted = {
                        let main_inv = self
                            .inventory()
                            .main_inventory
                            .read()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        main_inv
                            .iter()
                            .any(|stack| !stack.is_empty() && stack.has_enchantments())
                    };
                    if has_enchanted {
                        self.trigger_advancement_criterion(
                            Advancement::STORY_ENCHANT_ITEM,
                            "enchanted_item",
                        );
                    }
                }

                if !self.has_advancement(Advancement::NETHER_BREW_POTION) {
                    let potions = [&Item::POTION, &Item::SPLASH_POTION, &Item::LINGERING_POTION];
                    for item in potions {
                        if self.has_item_in_inventory(item) {
                            self.trigger_advancement_criterion(
                                Advancement::NETHER_BREW_POTION,
                                "potion",
                            );
                            break;
                        }
                    }
                }

                if !self.has_advancement(Advancement::NETHER_CREATE_BEACON) {
                    if self.has_item_in_inventory(&Item::BEACON) {
                        self.trigger_advancement_criterion(
                            Advancement::NETHER_CREATE_BEACON,
                            "beacon",
                        );
                    }
                }
                if !self.has_advancement(Advancement::NETHER_CREATE_FULL_BEACON) {
                    if self.has_item_in_inventory(&Item::BEACON) {
                        self.trigger_advancement_criterion(
                            Advancement::NETHER_CREATE_FULL_BEACON,
                            "beacon",
                        );
                    }
                }

                if !self.has_advancement(Advancement::END_ELYTRA) {
                    if self.has_item_in_inventory(&Item::ELYTRA) {
                        self.trigger_advancement_criterion(Advancement::END_ELYTRA, "elytra");
                    }
                }

                if !self.has_advancement(Advancement::END_DRAGON_EGG) {
                    if self.has_item_in_inventory(&Item::DRAGON_EGG) {
                        self.trigger_advancement_criterion(
                            Advancement::END_DRAGON_EGG,
                            "dragon_egg",
                        );
                    }
                }

                if !self.has_advancement(Advancement::END_DRAGON_BREATH) {
                    if self.has_item_in_inventory(&Item::DRAGON_BREATH) {
                        self.trigger_advancement_criterion(
                            Advancement::END_DRAGON_BREATH,
                            "dragon_breath",
                        );
                    }
                }

                if !self.has_advancement(Advancement::END_FIND_END_CITY) {
                    let city_items = [&Item::SHULKER_SHELL, &Item::CHORUS_FRUIT];
                    for item in city_items {
                        if self.has_item_in_inventory(item) {
                            self.trigger_advancement_criterion(
                                Advancement::END_FIND_END_CITY,
                                "in_city",
                            );
                            break;
                        }
                    }
                }

                if !self.has_advancement(Advancement::NETHER_EXPLORE_NETHER) {
                    let pos = self.position().to_block_pos();
                    let biome = self.world().get_zoomed_biome(&pos);
                    let biome_resource = format!("minecraft:{}", biome.registry_id);
                    let nether_biomes = [
                        "minecraft:basalt_deltas",
                        "minecraft:crimson_forest",
                        "minecraft:nether_wastes",
                        "minecraft:soul_sand_valley",
                        "minecraft:warped_forest",
                    ];
                    if nether_biomes.contains(&biome_resource.as_str()) {
                        self.trigger_advancement_criterion(
                            Advancement::NETHER_EXPLORE_NETHER,
                            &biome_resource,
                        );
                    }
                }

                if !self.has_advancement(Advancement::ADVENTURE_ADVENTURING_TIME) {
                    let pos = self.position().to_block_pos();
                    let biome = self.world().get_zoomed_biome(&pos);
                    let biome_resource = format!("minecraft:{}", biome.registry_id);
                    let overworld_biomes = [
                        "minecraft:badlands",
                        "minecraft:bamboo_jungle",
                        "minecraft:beach",
                        "minecraft:birch_forest",
                        "minecraft:cherry_grove",
                        "minecraft:cold_ocean",
                        "minecraft:dark_forest",
                        "minecraft:deep_cold_ocean",
                        "minecraft:deep_dark",
                        "minecraft:deep_frozen_ocean",
                        "minecraft:deep_lukewarm_ocean",
                        "minecraft:deep_ocean",
                        "minecraft:desert",
                        "minecraft:dripstone_caves",
                        "minecraft:eroded_badlands",
                        "minecraft:flower_forest",
                        "minecraft:forest",
                        "minecraft:frozen_ocean",
                        "minecraft:frozen_peaks",
                        "minecraft:frozen_river",
                        "minecraft:grove",
                        "minecraft:ice_spikes",
                        "minecraft:jagged_peaks",
                        "minecraft:jungle",
                        "minecraft:lukewarm_ocean",
                        "minecraft:lush_caves",
                        "minecraft:mangrove_swamp",
                        "minecraft:meadow",
                        "minecraft:mushroom_fields",
                        "minecraft:ocean",
                        "minecraft:old_growth_birch_forest",
                        "minecraft:old_growth_pine_taiga",
                        "minecraft:old_growth_spruce_taiga",
                        "minecraft:pale_garden",
                        "minecraft:plains",
                        "minecraft:river",
                        "minecraft:savanna",
                        "minecraft:savanna_plateau",
                        "minecraft:snowy_beach",
                        "minecraft:snowy_plains",
                        "minecraft:snowy_slopes",
                        "minecraft:snowy_taiga",
                        "minecraft:sparse_jungle",
                        "minecraft:stony_peaks",
                        "minecraft:stony_shore",
                        "minecraft:sulfur_caves",
                        "minecraft:sunflower_plains",
                        "minecraft:swamp",
                        "minecraft:taiga",
                        "minecraft:warm_ocean",
                        "minecraft:windswept_forest",
                        "minecraft:windswept_gravelly_hills",
                        "minecraft:windswept_hills",
                        "minecraft:windswept_savanna",
                        "minecraft:wooded_badlands",
                    ];
                    if overworld_biomes.contains(&biome_resource.as_str()) {
                        self.trigger_advancement_criterion(
                            Advancement::ADVENTURE_ADVENTURING_TIME,
                            &biome_resource,
                        );
                    }
                }
            }
            AdvancementTrigger::PlayerKilledEntity {
                entity_type_resource,
            } => {
                self.trigger_advancement_criterion(
                    Advancement::ADVENTURE_KILL_A_MOB,
                    &entity_type_resource,
                );
                self.trigger_advancement_criterion(
                    Advancement::ADVENTURE_KILL_ALL_MOBS,
                    &entity_type_resource,
                );
                if !self.has_advancement(Advancement::ADVENTURE_ROOT) {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_ROOT,
                        "killed_something",
                    );
                }
            }
            AdvancementTrigger::SleptInBed => {
                if !self.has_advancement(Advancement::ADVENTURE_SLEEP_IN_BED) {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_SLEEP_IN_BED,
                        "slept_in_bed",
                    );
                }
            }
            AdvancementTrigger::FishedItem { item_id } => {
                if !self.has_advancement(Advancement::HUSBANDRY_FISHY_BUSINESS) {
                    let fishes = [
                        ("minecraft:cod", "cod"),
                        ("minecraft:salmon", "salmon"),
                        ("minecraft:pufferfish", "pufferfish"),
                        ("minecraft:tropical_fish", "tropical_fish"),
                    ];
                    for (fish, criterion) in fishes {
                        if item_id == fish {
                            self.trigger_advancement_criterion(
                                Advancement::HUSBANDRY_FISHY_BUSINESS,
                                criterion,
                            );
                            break;
                        }
                    }
                }
            }
            AdvancementTrigger::PlacedBlock { block_id } => {
                if !self.has_advancement(Advancement::HUSBANDRY_PLANT_SEED) {
                    let seed_blocks = [
                        ("minecraft:wheat", "wheat"),
                        ("minecraft:pumpkin_stem", "pumpkin_stem"),
                        ("minecraft:melon_stem", "melon_stem"),
                        ("minecraft:beetroots", "beetroots"),
                        ("minecraft:nether_wart", "nether_wart"),
                        ("minecraft:torchflower", "torchflower"),
                        ("minecraft:pitcher_pod", "pitcher_pod"),
                    ];
                    for (block, criterion) in seed_blocks {
                        if block_id == block {
                            self.trigger_advancement_criterion(
                                Advancement::HUSBANDRY_PLANT_SEED,
                                criterion,
                            );
                            break;
                        }
                    }
                }
            }
            AdvancementTrigger::EnterDimension { dimension } => {
                if dimension == "the_nether" {
                    if !self.has_advancement(Advancement::STORY_ENTER_THE_NETHER) {
                        self.trigger_advancement_criterion(
                            Advancement::STORY_ENTER_THE_NETHER,
                            "entered_nether",
                        );
                    }
                    if !self.has_advancement(Advancement::NETHER_ROOT) {
                        self.trigger_advancement_criterion(
                            Advancement::NETHER_ROOT,
                            "entered_nether",
                        );
                    }
                } else if dimension == "the_end" {
                    if !self.has_advancement(Advancement::STORY_ENTER_THE_END) {
                        self.trigger_advancement_criterion(
                            Advancement::STORY_ENTER_THE_END,
                            "entered_end",
                        );
                    }
                    if !self.has_advancement(Advancement::END_ROOT) {
                        self.trigger_advancement_criterion(Advancement::END_ROOT, "entered_end");
                    }
                }
            }
            AdvancementTrigger::ConsumeItem { item_id } => {
                if !self.has_advancement(Advancement::HUSBANDRY_ROOT) {
                    self.trigger_advancement_criterion(
                        Advancement::HUSBANDRY_ROOT,
                        "consumed_item",
                    );
                }
                let food_name = item_id.strip_prefix("minecraft:").unwrap_or(&item_id);
                if Advancement::HUSBANDRY_BALANCED_DIET
                    .criteria
                    .contains(&food_name)
                {
                    self.trigger_advancement_criterion(
                        Advancement::HUSBANDRY_BALANCED_DIET,
                        food_name,
                    );
                }
            }
            AdvancementTrigger::PlayerKilled => {
                if !self.has_advancement(Advancement::ADVENTURE_ROOT) {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_ROOT,
                        "killed_by_something",
                    );
                }
            }
            AdvancementTrigger::DeflectedDamage => {
                if !self.has_advancement(Advancement::STORY_DEFLECT_ARROW) {
                    self.trigger_advancement_criterion(
                        Advancement::STORY_DEFLECT_ARROW,
                        "deflected_projectile",
                    );
                }
            }
            AdvancementTrigger::LaunchedEyeOfEnder => {
                if !self.has_advancement(Advancement::STORY_FOLLOW_ENDER_EYE) {
                    self.trigger_advancement_criterion(
                        Advancement::STORY_FOLLOW_ENDER_EYE,
                        "in_stronghold",
                    );
                }
            }
            AdvancementTrigger::GlowedSign => {
                if !self.has_advancement(Advancement::HUSBANDRY_MAKE_A_SIGN_GLOW) {
                    self.trigger_advancement_criterion(
                        Advancement::HUSBANDRY_MAKE_A_SIGN_GLOW,
                        "make_a_sign_glow",
                    );
                }
            }
            AdvancementTrigger::BredAnimal { parent_type } => {
                self.trigger_advancement_criterion(Advancement::HUSBANDRY_BREED_AN_ANIMAL, "bred");
                self.trigger_advancement_criterion(
                    Advancement::HUSBANDRY_BRED_ALL_ANIMALS,
                    &parent_type,
                );
            }
            AdvancementTrigger::EnterBlock { block } => {
                if block == &Block::END_GATEWAY
                    && !self.has_advancement(Advancement::END_ENTER_END_GATEWAY)
                {
                    self.trigger_advancement_criterion(
                        Advancement::END_ENTER_END_GATEWAY,
                        "entered_end_gateway",
                    );
                }
            }
            AdvancementTrigger::DealtOverkillDamage => {
                if !self.has_advancement(Advancement::ADVENTURE_OVEROVERKILL) {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_OVEROVERKILL,
                        "overoverkill",
                    );
                }
            }
            AdvancementTrigger::SniperDuel => {
                if !self.has_advancement(Advancement::ADVENTURE_SNIPER_DUEL) {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_SNIPER_DUEL,
                        "killed_skeleton",
                    );
                }
            }
            AdvancementTrigger::TwoBirdsOneArrow => {
                if !self.has_advancement(Advancement::ADVENTURE_TWO_BIRDS_ONE_ARROW) {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_TWO_BIRDS_ONE_ARROW,
                        "two_birds",
                    );
                }
            }
            AdvancementTrigger::Arbalistic => {
                if !self.has_advancement(Advancement::ADVENTURE_ARBALISTIC) {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_ARBALISTIC,
                        "arbalistic",
                    );
                }
            }
            AdvancementTrigger::Bullseye => {
                if !self.has_advancement(Advancement::ADVENTURE_BULLSEYE) {
                    self.trigger_advancement_criterion(Advancement::ADVENTURE_BULLSEYE, "bullseye");
                }
            }
            AdvancementTrigger::CuredZombieVillager => {
                if !self.has_advancement(Advancement::STORY_CURE_ZOMBIE_VILLAGER) {
                    self.trigger_advancement_criterion(
                        Advancement::STORY_CURE_ZOMBIE_VILLAGER,
                        "cured_zombie",
                    );
                }
            }
            AdvancementTrigger::FallFromHeight { start } => {
                // `DistancePredicate` compares float differences.
                let pos = self.living_entity.entity.pos.load();
                let distance_y = ((start.y - pos.y) as f32).abs();
                if !self.has_advancement(Advancement::ADVENTURE_FALL_FROM_WORLD_HEIGHT)
                    && start.y >= 319.0
                    && pos.y <= -59.0
                    && distance_y >= 379.0
                {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_FALL_FROM_WORLD_HEIGHT,
                        "fall_from_world_height",
                    );
                }
            }
            AdvancementTrigger::FallAfterExplosion { start, cause } => {
                let pos = self.living_entity.entity.pos.load();
                let distance_y = ((start.y - pos.y) as f32).abs();
                if !self.has_advancement(Advancement::ADVENTURE_WHO_NEEDS_ROCKETS)
                    && cause == Some(&EntityType::WIND_CHARGE)
                    && distance_y >= 7.0
                {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_WHO_NEEDS_ROCKETS,
                        "who_needs_rockets",
                    );
                }
            }
            AdvancementTrigger::TradedWithVillager => {
                if !self.has_advancement(Advancement::ADVENTURE_TRADE) {
                    self.trigger_advancement_criterion(Advancement::ADVENTURE_TRADE, "traded");
                }
                if self.living_entity.entity.pos.load().y >= 319.0
                    && !self.has_advancement(Advancement::ADVENTURE_TRADE_AT_WORLD_HEIGHT)
                {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_TRADE_AT_WORLD_HEIGHT,
                        "trade_at_world_height",
                    );
                }
            }
            AdvancementTrigger::AvoidVibration => {
                if !self.has_advancement(Advancement::ADVENTURE_AVOID_VIBRATION) {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_AVOID_VIBRATION,
                        "avoid_vibration",
                    );
                }
            }
            AdvancementTrigger::UsedTotem { item } => {
                if item == &Item::TOTEM_OF_UNDYING
                    && !self.has_advancement(Advancement::ADVENTURE_TOTEM_OF_UNDYING)
                {
                    self.trigger_advancement_criterion(
                        Advancement::ADVENTURE_TOTEM_OF_UNDYING,
                        "used_totem",
                    );
                }
            }
            AdvancementTrigger::ThrownItemPickedUpByEntity { item, entity } => {
                let picker = entity.get_entity().entity_type;
                // `distract_piglin`'s player condition: no piglin-safe armor in any armor slot.
                if picker == &EntityType::PIGLIN
                    && item.item.has_tag(&tag::Item::MINECRAFT_PIGLIN_LOVED)
                    && is_adult(entity.as_ref())
                    && !self.wears_piglin_safe_armor()
                    && !self.has_advancement(Advancement::NETHER_DISTRACT_PIGLIN)
                {
                    self.trigger_advancement_criterion(
                        Advancement::NETHER_DISTRACT_PIGLIN,
                        "distract_piglin",
                    );
                }
                if picker == &EntityType::SULFUR_CUBE
                    && item
                        .item
                        .has_tag(&tag::Item::MINECRAFT_SULFUR_CUBE_ARCHETYPE_EXPLOSIVE)
                    && is_adult(entity.as_ref())
                    && !self.has_advancement(Advancement::HUSBANDRY_UH_OH)
                {
                    self.trigger_advancement_criterion(
                        Advancement::HUSBANDRY_UH_OH,
                        "pick_up_dropped_tnt",
                    );
                }
            }
            AdvancementTrigger::ThrownItemPickedUpByPlayer { item: _, thrower } => {
                if thrower.get_entity().entity_type == &EntityType::ALLAY
                    && !self.has_advancement(Advancement::HUSBANDRY_ALLAY_DELIVER_ITEM_TO_PLAYER)
                {
                    self.trigger_advancement_criterion(
                        Advancement::HUSBANDRY_ALLAY_DELIVER_ITEM_TO_PLAYER,
                        "allay_deliver_item_to_player",
                    );
                }
            }
        }
    }

    fn wears_piglin_safe_armor(&self) -> bool {
        let equipment = self
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        [
            EquipmentSlot::HEAD,
            EquipmentSlot::CHEST,
            EquipmentSlot::LEGS,
            EquipmentSlot::FEET,
        ]
        .iter()
        .any(|slot| {
            equipment
                .get(slot)
                .item
                .has_tag(&tag::Item::MINECRAFT_PIGLIN_SAFE_ARMOR)
        })
    }
}
