/* This file is generated. Do not edit manually. */
use crate::enchantment::Enchantment;
#[derive(Clone, Copy, PartialEq)]
pub enum EnchantmentProviderKind {
    Single {
        enchantment: &'static Enchantment,
        level: i32,
    },
    ByCostWithDifficulty {
        enchantments: &'static [&'static Enchantment],
        min_cost: i32,
        max_cost_span: i32,
    },
    ByCost {
        enchantments: &'static [&'static Enchantment],
        cost: i32,
    },
}
impl core::fmt::Debug for EnchantmentProviderKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Single { enchantment, level } => f
                .debug_struct("Single")
                .field("enchantment", &enchantment.registry_key)
                .field("level", level)
                .finish(),
            Self::ByCostWithDifficulty {
                enchantments,
                min_cost,
                max_cost_span,
            } => {
                let keys: Vec<&str> = enchantments.iter().map(|e| e.registry_key).collect();
                f.debug_struct("ByCostWithDifficulty")
                    .field("enchantments", &keys)
                    .field("min_cost", min_cost)
                    .field("max_cost_span", max_cost_span)
                    .finish()
            }
            Self::ByCost { enchantments, cost } => {
                let keys: Vec<&str> = enchantments.iter().map(|e| e.registry_key).collect();
                f.debug_struct("ByCost")
                    .field("enchantments", &keys)
                    .field("cost", cost)
                    .finish()
            }
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
pub struct EnchantmentProvider {
    pub name: &'static str,
    pub kind: EnchantmentProviderKind,
}
impl core::fmt::Debug for EnchantmentProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EnchantmentProvider")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .finish()
    }
}
impl EnchantmentProvider {
    pub const ENDERMAN_LOOT_DROP: Self = Self {
        name: "minecraft:enderman_loot_drop",
        kind: EnchantmentProviderKind::Single {
            enchantment: &Enchantment::SILK_TOUCH,
            level: 1i32,
        },
    };
    pub const MOB_SPAWN_EQUIPMENT: Self = Self {
        name: "minecraft:mob_spawn_equipment",
        kind: EnchantmentProviderKind::ByCostWithDifficulty {
            enchantments: &[
                &Enchantment::PROTECTION,
                &Enchantment::FIRE_PROTECTION,
                &Enchantment::FEATHER_FALLING,
                &Enchantment::BLAST_PROTECTION,
                &Enchantment::PROJECTILE_PROTECTION,
                &Enchantment::RESPIRATION,
                &Enchantment::AQUA_AFFINITY,
                &Enchantment::THORNS,
                &Enchantment::DEPTH_STRIDER,
                &Enchantment::SHARPNESS,
                &Enchantment::SMITE,
                &Enchantment::BANE_OF_ARTHROPODS,
                &Enchantment::KNOCKBACK,
                &Enchantment::FIRE_ASPECT,
                &Enchantment::LOOTING,
                &Enchantment::SWEEPING_EDGE,
                &Enchantment::EFFICIENCY,
                &Enchantment::SILK_TOUCH,
                &Enchantment::UNBREAKING,
                &Enchantment::FORTUNE,
                &Enchantment::POWER,
                &Enchantment::PUNCH,
                &Enchantment::FLAME,
                &Enchantment::INFINITY,
                &Enchantment::LUCK_OF_THE_SEA,
                &Enchantment::LURE,
                &Enchantment::LOYALTY,
                &Enchantment::IMPALING,
                &Enchantment::RIPTIDE,
                &Enchantment::CHANNELING,
                &Enchantment::MULTISHOT,
                &Enchantment::QUICK_CHARGE,
                &Enchantment::PIERCING,
                &Enchantment::DENSITY,
                &Enchantment::BREACH,
                &Enchantment::LUNGE,
            ],
            min_cost: 5i32,
            max_cost_span: 17i32,
        },
    };
    pub const PILLAGER_SPAWN_CROSSBOW: Self = Self {
        name: "minecraft:pillager_spawn_crossbow",
        kind: EnchantmentProviderKind::Single {
            enchantment: &Enchantment::PIERCING,
            level: 1i32,
        },
    };
    pub const RAID_PILLAGER_POST_WAVE_3: Self = Self {
        name: "minecraft:raid/pillager_post_wave_3",
        kind: EnchantmentProviderKind::Single {
            enchantment: &Enchantment::QUICK_CHARGE,
            level: 1i32,
        },
    };
    pub const RAID_PILLAGER_POST_WAVE_5: Self = Self {
        name: "minecraft:raid/pillager_post_wave_5",
        kind: EnchantmentProviderKind::Single {
            enchantment: &Enchantment::QUICK_CHARGE,
            level: 2i32,
        },
    };
    pub const RAID_VINDICATOR: Self = Self {
        name: "minecraft:raid/vindicator",
        kind: EnchantmentProviderKind::Single {
            enchantment: &Enchantment::SHARPNESS,
            level: 1i32,
        },
    };
    pub const RAID_VINDICATOR_POST_WAVE_5: Self = Self {
        name: "minecraft:raid/vindicator_post_wave_5",
        kind: EnchantmentProviderKind::Single {
            enchantment: &Enchantment::SHARPNESS,
            level: 2i32,
        },
    };
    pub const ALL: &'static [Self] = &[
        Self::ENDERMAN_LOOT_DROP,
        Self::MOB_SPAWN_EQUIPMENT,
        Self::PILLAGER_SPAWN_CROSSBOW,
        Self::RAID_PILLAGER_POST_WAVE_3,
        Self::RAID_PILLAGER_POST_WAVE_5,
        Self::RAID_VINDICATOR,
        Self::RAID_VINDICATOR_POST_WAVE_5,
    ];
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
    #[must_use]
    pub const fn kind(&self) -> &EnchantmentProviderKind {
        &self.kind
    }
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::match_same_arms)]
    #[must_use]
    pub fn from_name(name: &str) -> Option<&'static Self> {
        match name {
            "minecraft:enderman_loot_drop" | "enderman_loot_drop" => {
                Some(&Self::ENDERMAN_LOOT_DROP)
            }
            "minecraft:mob_spawn_equipment" | "mob_spawn_equipment" => {
                Some(&Self::MOB_SPAWN_EQUIPMENT)
            }
            "minecraft:pillager_spawn_crossbow" | "pillager_spawn_crossbow" => {
                Some(&Self::PILLAGER_SPAWN_CROSSBOW)
            }
            "minecraft:raid/pillager_post_wave_3" | "raid/pillager_post_wave_3" => {
                Some(&Self::RAID_PILLAGER_POST_WAVE_3)
            }
            "minecraft:raid/pillager_post_wave_5" | "raid/pillager_post_wave_5" => {
                Some(&Self::RAID_PILLAGER_POST_WAVE_5)
            }
            "minecraft:raid/vindicator" | "raid/vindicator" => Some(&Self::RAID_VINDICATOR),
            "minecraft:raid/vindicator_post_wave_5" | "raid/vindicator_post_wave_5" => {
                Some(&Self::RAID_VINDICATOR_POST_WAVE_5)
            }
            _ => None,
        }
    }
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::match_same_arms)]
    #[must_use]
    pub fn get_raw_json(name: &str) -> Option<&'static str> {
        match name {
            "minecraft:enderman_loot_drop" | "enderman_loot_drop" => Some(include_str!(
                "../../../../assets/datapack/data/minecraft/enchantment_provider/enderman_loot_drop.json"
            )),
            "minecraft:mob_spawn_equipment" | "mob_spawn_equipment" => Some(include_str!(
                "../../../../assets/datapack/data/minecraft/enchantment_provider/mob_spawn_equipment.json"
            )),
            "minecraft:pillager_spawn_crossbow" | "pillager_spawn_crossbow" => Some(include_str!(
                "../../../../assets/datapack/data/minecraft/enchantment_provider/pillager_spawn_crossbow.json"
            )),
            "minecraft:raid/pillager_post_wave_3" | "raid/pillager_post_wave_3" => {
                Some(include_str!(
                    "../../../../assets/datapack/data/minecraft/enchantment_provider/raid/pillager_post_wave_3.json"
                ))
            }
            "minecraft:raid/pillager_post_wave_5" | "raid/pillager_post_wave_5" => {
                Some(include_str!(
                    "../../../../assets/datapack/data/minecraft/enchantment_provider/raid/pillager_post_wave_5.json"
                ))
            }
            "minecraft:raid/vindicator" | "raid/vindicator" => Some(include_str!(
                "../../../../assets/datapack/data/minecraft/enchantment_provider/raid/vindicator.json"
            )),
            "minecraft:raid/vindicator_post_wave_5" | "raid/vindicator_post_wave_5" => {
                Some(include_str!(
                    "../../../../assets/datapack/data/minecraft/enchantment_provider/raid/vindicator_post_wave_5.json"
                ))
            }
            _ => None,
        }
    }
    #[must_use]
    pub const fn all() -> &'static [Self] {
        Self::ALL
    }
    #[must_use]
    pub const fn all_provider_names() -> &'static [&'static str] {
        &[
            "minecraft:enderman_loot_drop",
            "minecraft:mob_spawn_equipment",
            "minecraft:pillager_spawn_crossbow",
            "minecraft:raid/pillager_post_wave_3",
            "minecraft:raid/pillager_post_wave_5",
            "minecraft:raid/vindicator",
            "minecraft:raid/vindicator_post_wave_5",
        ]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_providers_count() {
        assert!(EnchantmentProvider::ALL.len() >= 7);
    }
    #[test]
    fn lookup_by_name() {
        assert_eq!(
            EnchantmentProvider::from_name("minecraft:mob_spawn_equipment"),
            Some(&EnchantmentProvider::MOB_SPAWN_EQUIPMENT)
        );
        assert_eq!(
            EnchantmentProvider::from_name("mob_spawn_equipment"),
            Some(&EnchantmentProvider::MOB_SPAWN_EQUIPMENT)
        );
        assert_eq!(
            EnchantmentProvider::from_name("enderman_loot_drop"),
            Some(&EnchantmentProvider::ENDERMAN_LOOT_DROP)
        );
        assert_eq!(
            EnchantmentProvider::from_name("raid/vindicator"),
            Some(&EnchantmentProvider::RAID_VINDICATOR)
        );
        assert_eq!(
            EnchantmentProvider::from_name("minecraft:raid/pillager_post_wave_5"),
            Some(&EnchantmentProvider::RAID_PILLAGER_POST_WAVE_5)
        );
    }
    #[test]
    fn raw_json_retrieval() {
        assert!(EnchantmentProvider::get_raw_json("mob_spawn_equipment").is_some());
        assert!(EnchantmentProvider::get_raw_json("minecraft:enderman_loot_drop").is_some());
        assert!(EnchantmentProvider::get_raw_json("nonexistent").is_none());
    }
}
