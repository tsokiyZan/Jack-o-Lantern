//! Vanilla's state-independent mob sound overrides, by entity type.
//!
//! These are the `getAmbientSound`, `getAmbientSoundInterval`, `getSoundVolume`, step,
//! swim, splash, `getMovementEmission` and `nextStep` overrides that return a constant.
//! Mobs whose override reads their state override the `Mob` method instead.

use crate::entity::movement_emission::MovementEmission;
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

/// Vanilla `playStepSound` overrides that play one fixed sound, with its volume (pitch 1).
/// Zombies and skeletons go through their `getStepSound`.
const STEP_SOUNDS: &[(&EntityType, (Sound, f32))] = &[
    (&EntityType::ARMADILLO, (Sound::EntityArmadilloStep, 0.15)),
    (&EntityType::BOGGED, (Sound::EntityBoggedStep, 0.15)),
    (&EntityType::CAVE_SPIDER, (Sound::EntitySpiderStep, 0.15)),
    (&EntityType::CREAKING, (Sound::EntityCreakingStep, 0.15)),
    (&EntityType::DROWNED, (Sound::EntityDrownedStep, 0.15)),
    (&EntityType::ENDERMITE, (Sound::EntityEndermiteStep, 0.15)),
    (&EntityType::FROG, (Sound::EntityFrogStep, 0.15)),
    (&EntityType::GOAT, (Sound::EntityGoatStep, 0.15)),
    (&EntityType::HOGLIN, (Sound::EntityHoglinStep, 0.15)),
    (&EntityType::HUSK, (Sound::EntityHuskStep, 0.15)),
    (&EntityType::IRON_GOLEM, (Sound::EntityIronGolemStep, 1.0)),
    (&EntityType::LLAMA, (Sound::EntityLlamaStep, 0.15)),
    (&EntityType::PANDA, (Sound::EntityPandaStep, 0.15)),
    (&EntityType::PARCHED, (Sound::EntityParchedStep, 0.15)),
    (&EntityType::PARROT, (Sound::EntityParrotStep, 0.15)),
    (&EntityType::PIGLIN, (Sound::EntityPiglinStep, 0.15)),
    (
        &EntityType::PIGLIN_BRUTE,
        (Sound::EntityPiglinBruteStep, 0.15),
    ),
    (&EntityType::POLAR_BEAR, (Sound::EntityPolarBearStep, 0.15)),
    (&EntityType::RAVAGER, (Sound::EntityRavagerStep, 0.15)),
    (&EntityType::SHEEP, (Sound::EntitySheepStep, 0.15)),
    (&EntityType::SILVERFISH, (Sound::EntitySilverfishStep, 0.15)),
    (&EntityType::SKELETON, (Sound::EntitySkeletonStep, 0.15)),
    (&EntityType::SNIFFER, (Sound::EntitySnifferStep, 0.15)),
    (&EntityType::SPIDER, (Sound::EntitySpiderStep, 0.15)),
    (&EntityType::STRAY, (Sound::EntityStrayStep, 0.15)),
    (&EntityType::TRADER_LLAMA, (Sound::EntityLlamaStep, 0.15)),
    (&EntityType::WARDEN, (Sound::EntityWardenStep, 10.0)),
    (
        &EntityType::WITHER_SKELETON,
        (Sound::EntityWitherSkeletonStep, 0.15),
    ),
    (&EntityType::ZOGLIN, (Sound::EntityZoglinStep, 0.15)),
    (&EntityType::ZOMBIE, (Sound::EntityZombieStep, 0.15)),
    (
        &EntityType::ZOMBIE_VILLAGER,
        (Sound::EntityZombieVillagerStep, 0.15),
    ),
    (
        &EntityType::ZOMBIFIED_PIGLIN,
        (Sound::EntityZombieStep, 0.15),
    ),
];

/// Vanilla's empty `playStepSound` overrides.
const SILENT_STEPS: &[&EntityType] = &[
    &EntityType::ALLAY,
    &EntityType::BEE,
    &EntityType::COD,
    &EntityType::HAPPY_GHAST,
    &EntityType::NAUTILUS,
    &EntityType::PUFFERFISH,
    &EntityType::SALMON,
    &EntityType::TADPOLE,
    &EntityType::TROPICAL_FISH,
    &EntityType::ZOMBIE_NAUTILUS,
];

/// Every vanilla `Monster`, which swims and splashes with the hostile sounds.
const MONSTERS: &[&EntityType] = &[
    &EntityType::BLAZE,
    &EntityType::BOGGED,
    &EntityType::BREEZE,
    &EntityType::CAVE_SPIDER,
    &EntityType::CREAKING,
    &EntityType::CREEPER,
    &EntityType::DROWNED,
    &EntityType::ELDER_GUARDIAN,
    &EntityType::ENDERMAN,
    &EntityType::ENDERMITE,
    &EntityType::EVOKER,
    &EntityType::GIANT,
    &EntityType::GUARDIAN,
    &EntityType::HUSK,
    &EntityType::ILLUSIONER,
    &EntityType::PARCHED,
    &EntityType::PIGLIN,
    &EntityType::PIGLIN_BRUTE,
    &EntityType::PILLAGER,
    &EntityType::RAVAGER,
    &EntityType::SILVERFISH,
    &EntityType::SKELETON,
    &EntityType::SPIDER,
    &EntityType::STRAY,
    &EntityType::VEX,
    &EntityType::VINDICATOR,
    &EntityType::WARDEN,
    &EntityType::WITCH,
    &EntityType::WITHER,
    &EntityType::WITHER_SKELETON,
    &EntityType::ZOGLIN,
    &EntityType::ZOMBIE,
    &EntityType::ZOMBIE_VILLAGER,
    &EntityType::ZOMBIFIED_PIGLIN,
];

/// Vanilla `getSwimSound` overrides that return one fixed sound, besides `Monster`'s.
const SWIM_SOUNDS: &[(&EntityType, Sound)] = &[
    (&EntityType::AXOLOTL, Sound::EntityAxolotlSwim),
    (&EntityType::COD, Sound::EntityFishSwim),
    (&EntityType::DOLPHIN, Sound::EntityDolphinSwim),
    (&EntityType::DROWNED, Sound::EntityDrownedSwim),
    (&EntityType::HOGLIN, Sound::EntityHostileSwim),
    (&EntityType::PUFFERFISH, Sound::EntityFishSwim),
    (&EntityType::SALMON, Sound::EntityFishSwim),
    (&EntityType::TADPOLE, Sound::EntityFishSwim),
    (&EntityType::TROPICAL_FISH, Sound::EntityFishSwim),
    (&EntityType::TURTLE, Sound::EntityTurtleSwim),
    (
        &EntityType::ZOMBIE_NAUTILUS,
        Sound::EntityZombieNautilusSwim,
    ),
];

/// Vanilla `getSwimSplashSound` overrides, besides `Monster`'s.
const SWIM_SPLASH_SOUNDS: &[(&EntityType, Sound)] = &[
    (&EntityType::AXOLOTL, Sound::EntityAxolotlSplash),
    (&EntityType::DOLPHIN, Sound::EntityDolphinSplash),
    (&EntityType::HOGLIN, Sound::EntityHostileSplash),
];

/// Vanilla `getMovementEmission` overrides on mobs.
const MOVEMENT_EMISSIONS: &[(&EntityType, MovementEmission)] = &[
    (&EntityType::BAT, MovementEmission::Events),
    (&EntityType::BREEZE, MovementEmission::Events),
    (&EntityType::ELDER_GUARDIAN, MovementEmission::Events),
    (&EntityType::ENDERMITE, MovementEmission::Events),
    (&EntityType::GLOW_SQUID, MovementEmission::Events),
    (&EntityType::GUARDIAN, MovementEmission::Events),
    (&EntityType::SHULKER, MovementEmission::None),
    (&EntityType::SILVERFISH, MovementEmission::Events),
    (&EntityType::SQUID, MovementEmission::Events),
];

/// Vanilla `nextStep` overrides: `moveDist` plus this, where the default rounds up.
const NEXT_STEP_DISTANCES: &[(&EntityType, f32)] = &[
    (&EntityType::STRIDER, 0.6),
    (&EntityType::TURTLE, 0.15),
    (&EntityType::WARDEN, 0.55),
];

/// How a mob's steps sound when it has no state-dependent `playStepSound` override.
pub enum StepSound {
    /// Vanilla `Entity.playStepSound`: the block's own step sound.
    Block,
    Silent,
    Fixed(Sound, f32),
}

fn is_listed(list: &[&EntityType], entity_type: &EntityType) -> bool {
    list.iter().any(|candidate| candidate.id == entity_type.id)
}

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

#[must_use]
pub fn step_sound(entity_type: &EntityType) -> StepSound {
    if let Some((sound, volume)) = lookup(STEP_SOUNDS, entity_type) {
        StepSound::Fixed(sound, volume)
    } else if is_listed(SILENT_STEPS, entity_type) {
        StepSound::Silent
    } else {
        StepSound::Block
    }
}

#[must_use]
pub fn swim_sound(entity_type: &EntityType) -> Sound {
    lookup(SWIM_SOUNDS, entity_type).unwrap_or_else(|| {
        if is_listed(MONSTERS, entity_type) {
            Sound::EntityHostileSwim
        } else {
            Sound::EntityGenericSwim
        }
    })
}

#[must_use]
pub fn swim_splash_sound(entity_type: &EntityType) -> Sound {
    lookup(SWIM_SPLASH_SOUNDS, entity_type).unwrap_or_else(|| {
        if is_listed(MONSTERS, entity_type) {
            Sound::EntityHostileSplash
        } else {
            Sound::EntityGenericSplash
        }
    })
}

#[must_use]
pub fn movement_emission(entity_type: &EntityType) -> MovementEmission {
    lookup(MOVEMENT_EMISSIONS, entity_type).unwrap_or(MovementEmission::All)
}

#[must_use]
pub fn next_step_distance(entity_type: &EntityType) -> Option<f32> {
    lookup(NEXT_STEP_DISTANCES, entity_type)
}
