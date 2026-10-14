//! Vanilla's state-independent mob sound overrides, by entity type.
//!
//! These are the `getAmbientSound`, `getAmbientSoundInterval` and `getSoundVolume`
//! overrides that return a constant. Mobs whose override reads their state override the
//! `Mob` method instead.

use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::{Sound, SoundCategory};

/// Vanilla `Mob.getAmbientSoundInterval`.
pub const DEFAULT_AMBIENT_SOUND_INTERVAL: i32 = 80;

/// The vanilla `getAmbientSound` overrides that return one fixed sound.
const AMBIENT_SOUNDS: &[(&EntityType, Sound)] = &[
    (&EntityType::BLAZE, Sound::EntityBlazeAmbient),
    (&EntityType::BOGGED, Sound::EntityBoggedAmbient),
    (&EntityType::CAMEL, Sound::EntityCamelAmbient),
    (&EntityType::CAMEL_HUSK, Sound::EntityCamelHuskAmbient),
    (&EntityType::CAVE_SPIDER, Sound::EntitySpiderAmbient),
    (&EntityType::COD, Sound::EntityCodAmbient),
    (&EntityType::DONKEY, Sound::EntityDonkeyAmbient),
    (&EntityType::ENDER_DRAGON, Sound::EntityEnderDragonAmbient),
    (&EntityType::ENDERMITE, Sound::EntityEndermiteAmbient),
    (&EntityType::EVOKER, Sound::EntityEvokerAmbient),
    (&EntityType::FROG, Sound::EntityFrogAmbient),
    (&EntityType::GHAST, Sound::EntityGhastAmbient),
    (&EntityType::GLOW_SQUID, Sound::EntityGlowSquidAmbient),
    (&EntityType::HUSK, Sound::EntityHuskAmbient),
    (&EntityType::ILLUSIONER, Sound::EntityIllusionerAmbient),
    (&EntityType::LLAMA, Sound::EntityLlamaAmbient),
    (&EntityType::MULE, Sound::EntityMuleAmbient),
    (&EntityType::OCELOT, Sound::EntityOcelotAmbient),
    (&EntityType::PARCHED, Sound::EntityParchedAmbient),
    (&EntityType::PHANTOM, Sound::EntityPhantomAmbient),
    (&EntityType::PIGLIN_BRUTE, Sound::EntityPiglinBruteAmbient),
    (&EntityType::PILLAGER, Sound::EntityPillagerAmbient),
    (&EntityType::RABBIT, Sound::EntityRabbitAmbient),
    (&EntityType::RAVAGER, Sound::EntityRavagerAmbient),
    (&EntityType::SALMON, Sound::EntitySalmonAmbient),
    (&EntityType::SHEEP, Sound::EntitySheepAmbient),
    (&EntityType::SHULKER, Sound::EntityShulkerAmbient),
    (&EntityType::SILVERFISH, Sound::EntitySilverfishAmbient),
    (&EntityType::SKELETON, Sound::EntitySkeletonAmbient),
    (&EntityType::SNOW_GOLEM, Sound::EntitySnowGolemAmbient),
    (&EntityType::SPIDER, Sound::EntitySpiderAmbient),
    (&EntityType::SQUID, Sound::EntitySquidAmbient),
    (&EntityType::STRAY, Sound::EntityStrayAmbient),
    (&EntityType::TRADER_LLAMA, Sound::EntityLlamaAmbient),
    (&EntityType::TROPICAL_FISH, Sound::EntityTropicalFishAmbient),
    (&EntityType::VEX, Sound::EntityVexAmbient),
    (&EntityType::VINDICATOR, Sound::EntityVindicatorAmbient),
    (&EntityType::WITCH, Sound::EntityWitchAmbient),
    (&EntityType::WITHER, Sound::EntityWitherAmbient),
    (
        &EntityType::WITHER_SKELETON,
        Sound::EntityWitherSkeletonAmbient,
    ),
    (&EntityType::ZOMBIE, Sound::EntityZombieAmbient),
    (&EntityType::ZOMBIE_HORSE, Sound::EntityZombieHorseAmbient),
    (
        &EntityType::ZOMBIE_VILLAGER,
        Sound::EntityZombieVillagerAmbient,
    ),
];

/// Vanilla `getAmbientSoundInterval` overrides: `Animal`, `WaterAnimal`,
/// `AgeableWaterCreature` and `AbstractGolem` use 120 and `AbstractHorse` 400.
const AMBIENT_SOUND_INTERVALS: &[(&EntityType, i32)] = &[
    (&EntityType::ARMADILLO, 120),
    (&EntityType::AXOLOTL, 120),
    (&EntityType::BEE, 120),
    (&EntityType::CAT, 120),
    (&EntityType::CHICKEN, 120),
    (&EntityType::COD, 120),
    (&EntityType::COPPER_GOLEM, 120),
    (&EntityType::COW, 120),
    (&EntityType::DOLPHIN, 120),
    (&EntityType::FOX, 120),
    (&EntityType::FROG, 120),
    (&EntityType::GLOW_SQUID, 120),
    (&EntityType::GOAT, 120),
    (&EntityType::HOGLIN, 120),
    (&EntityType::IRON_GOLEM, 120),
    (&EntityType::MOOSHROOM, 120),
    (&EntityType::NAUTILUS, 120),
    (&EntityType::PANDA, 120),
    (&EntityType::PARROT, 120),
    (&EntityType::PIG, 120),
    (&EntityType::POLAR_BEAR, 120),
    (&EntityType::PUFFERFISH, 120),
    (&EntityType::RABBIT, 120),
    (&EntityType::SALMON, 120),
    (&EntityType::SHEEP, 120),
    (&EntityType::SHULKER, 120),
    (&EntityType::SNIFFER, 120),
    (&EntityType::SNOW_GOLEM, 120),
    (&EntityType::SQUID, 120),
    (&EntityType::STRIDER, 120),
    (&EntityType::TADPOLE, 120),
    (&EntityType::TROPICAL_FISH, 120),
    (&EntityType::WOLF, 120),
    (&EntityType::ZOMBIE_NAUTILUS, 120),
    (&EntityType::CAMEL, 400),
    (&EntityType::CAMEL_HUSK, 400),
    (&EntityType::DONKEY, 400),
    (&EntityType::HORSE, 400),
    (&EntityType::LLAMA, 400),
    (&EntityType::MULE, 400),
    (&EntityType::SKELETON_HORSE, 400),
    (&EntityType::TRADER_LLAMA, 400),
    (&EntityType::ZOMBIE_HORSE, 400),
    (&EntityType::ELDER_GUARDIAN, 160),
    (&EntityType::GUARDIAN, 160),
    (&EntityType::OCELOT, 900),
    (&EntityType::TURTLE, 200),
];

/// Vanilla `getSoundVolume` overrides that return a constant. Cube mobs scale with
/// their size and happy ghasts with their age, so those override `Mob::sound_volume`.
const SOUND_VOLUMES: &[(&EntityType, f32)] = &[
    (&EntityType::ALLAY, 0.4),
    (&EntityType::BEE, 0.4),
    (&EntityType::COW, 0.4),
    (&EntityType::GLOW_SQUID, 0.4),
    (&EntityType::MOOSHROOM, 0.4),
    (&EntityType::SQUID, 0.4),
    (&EntityType::WOLF, 0.4),
    (&EntityType::BAT, 0.1),
    (&EntityType::CAMEL, 0.8),
    (&EntityType::CAMEL_HUSK, 0.8),
    (&EntityType::DONKEY, 0.8),
    (&EntityType::HORSE, 0.8),
    (&EntityType::LLAMA, 0.8),
    (&EntityType::MULE, 0.8),
    (&EntityType::SKELETON_HORSE, 0.8),
    (&EntityType::TRADER_LLAMA, 0.8),
    (&EntityType::ZOMBIE_HORSE, 0.8),
    (&EntityType::ENDER_DRAGON, 5.0),
    (&EntityType::GHAST, 5.0),
    (&EntityType::WARDEN, 4.0),
];

fn lookup<T: Copy>(table: &[(&EntityType, T)], entity_type: &EntityType) -> Option<T> {
    table
        .iter()
        .find(|(candidate, _)| candidate.id == entity_type.id)
        .map(|(_, value)| *value)
}

#[must_use]
pub fn ambient_sound(entity_type: &EntityType) -> Option<Sound> {
    lookup(AMBIENT_SOUNDS, entity_type)
}

#[must_use]
pub fn ambient_sound_interval(entity_type: &EntityType) -> i32 {
    lookup(AMBIENT_SOUND_INTERVALS, entity_type).unwrap_or(DEFAULT_AMBIENT_SOUND_INTERVAL)
}

#[must_use]
pub fn sound_volume(entity_type: &EntityType) -> f32 {
    lookup(SOUND_VOLUMES, entity_type).unwrap_or(1.0)
}

/// Vanilla `getSoundSource`: every `Enemy` plays in the hostile category, everything
/// else in the neutral one. Killer rabbits override it.
#[must_use]
pub const fn sound_source(entity_type: &EntityType) -> SoundCategory {
    if entity_type.is_enemy {
        SoundCategory::Hostile
    } else {
        SoundCategory::Neutral
    }
}
