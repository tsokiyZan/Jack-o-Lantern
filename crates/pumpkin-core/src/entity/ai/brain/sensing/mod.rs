use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use rand::{Rng, RngExt};

use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::passive::armadillo;
use crate::entity::{Entity, EntityBase};
use crate::world::World;

use super::memory::{MemoryModuleId, types};
use super::{BrainTick, VisibilityContext};

pub mod adult;
pub mod axolotl_attackables;
pub mod breeze_attack_entity;
pub mod dummy;
pub mod frog_attackables;
pub mod hoglin_specific;
pub mod hurt_by;
pub mod is_in_water;
pub mod mob_sensor;
pub mod nearest_items;
pub mod nearest_living_entities;
pub mod nearest_players;
pub mod piglin_brute_specific;
pub mod piglin_specific;
pub mod tempting;

pub use adult::{AdultSensor, AdultSensorAnyType};
pub use axolotl_attackables::AxolotlAttackablesSensor;
pub use breeze_attack_entity::BreezeAttackEntitySensor;
pub use dummy::DummySensor;
pub use frog_attackables::FrogAttackablesSensor;
pub use hoglin_specific::HoglinSpecificSensor;
pub use hurt_by::HurtBySensor;
pub use is_in_water::IsInWaterSensor;
pub use mob_sensor::MobSensor;
pub use nearest_items::NearestItemSensor;
pub use nearest_living_entities::NearestLivingEntitySensor;
pub use nearest_players::PlayerSensor;
pub use piglin_brute_specific::PiglinBruteSpecificSensor;
pub use piglin_specific::PiglinSpecificSensor;
pub use tempting::TemptingSensor;

pub const DEFAULT_SCAN_RATE: i32 = 20;
/// Vanilla `SensorType.ARMADILLO_SCARE_DETECTED`'s scan rate.
const ARMADILLO_SCARE_DETECTED_SCAN_RATE: i32 = 5;

pub trait Sensor: Send + Sync {
    fn requires(&self) -> &'static [MemoryModuleId];
    fn do_tick(&mut self, tick: &mut BrainTick<'_>);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SensorType {
    Dummy,
    NearestLivingEntities,
    NearestPlayers,
    NearestItems,
    HurtBy,
    PiglinSpecific,
    PiglinBruteSpecific,
    HoglinSpecific,
    NearestAdult,
    NearestAdultAnyType,
    AxolotlAttackables,
    FoodTemptations,
    FrogTemptations,
    NautilusTemptations,
    FrogAttackables,
    IsInWater,
    ArmadilloScareDetected,
    BreezeAttackEntitySensor,
}

impl SensorType {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Dummy => "minecraft:dummy",
            Self::NearestLivingEntities => "minecraft:nearest_living_entities",
            Self::NearestPlayers => "minecraft:nearest_players",
            Self::NearestItems => "minecraft:nearest_items",
            Self::HurtBy => "minecraft:hurt_by",
            Self::PiglinSpecific => "minecraft:piglin_specific_sensor",
            Self::PiglinBruteSpecific => "minecraft:piglin_brute_specific_sensor",
            Self::HoglinSpecific => "minecraft:hoglin_specific_sensor",
            Self::NearestAdult => "minecraft:nearest_adult",
            Self::NearestAdultAnyType => "minecraft:nearest_adult_any_type",
            Self::AxolotlAttackables => "minecraft:axolotl_attackables",
            Self::FoodTemptations => "minecraft:food_temptations",
            Self::FrogTemptations => "minecraft:frog_temptations",
            Self::NautilusTemptations => "minecraft:nautilus_temptations",
            Self::FrogAttackables => "minecraft:frog_attackables",
            Self::IsInWater => "minecraft:is_in_water",
            Self::ArmadilloScareDetected => "minecraft:armadillo_scare_detected",
            Self::BreezeAttackEntitySensor => "minecraft:breeze_attack_entity_sensor",
        }
    }

    #[must_use]
    pub const fn scan_rate(self) -> i32 {
        match self {
            Self::Dummy
            | Self::NearestLivingEntities
            | Self::NearestPlayers
            | Self::NearestItems
            | Self::HurtBy
            | Self::PiglinSpecific
            | Self::PiglinBruteSpecific
            | Self::HoglinSpecific
            | Self::NearestAdult
            | Self::NearestAdultAnyType
            | Self::AxolotlAttackables
            | Self::FoodTemptations
            | Self::FrogTemptations
            | Self::NautilusTemptations
            | Self::FrogAttackables
            | Self::IsInWater
            | Self::BreezeAttackEntitySensor => DEFAULT_SCAN_RATE,
            Self::ArmadilloScareDetected => ARMADILLO_SCARE_DETECTED_SCAN_RATE,
        }
    }

    fn create_sensor(self) -> Box<dyn Sensor> {
        match self {
            Self::Dummy => Box::new(DummySensor),
            Self::NearestLivingEntities => Box::new(NearestLivingEntitySensor),
            Self::NearestPlayers => Box::new(PlayerSensor),
            Self::NearestItems => Box::new(NearestItemSensor),
            Self::HurtBy => Box::new(HurtBySensor),
            Self::PiglinSpecific => Box::new(PiglinSpecificSensor),
            Self::PiglinBruteSpecific => Box::new(PiglinBruteSpecificSensor),
            Self::HoglinSpecific => Box::new(HoglinSpecificSensor),
            Self::NearestAdult => Box::new(AdultSensor),
            Self::NearestAdultAnyType => Box::new(AdultSensorAnyType),
            Self::AxolotlAttackables => Box::new(AxolotlAttackablesSensor),
            Self::FoodTemptations => Box::new(TemptingSensor::for_animal()),
            Self::FrogTemptations => Box::new(TemptingSensor::new(|_, stack| {
                crate::entity::passive::frog_ai::get_temptations(stack)
            })),
            Self::NautilusTemptations => Box::new(TemptingSensor::new(|_, stack| {
                crate::entity::passive::nautilus_ai::get_temptations(stack)
            })),
            Self::FrogAttackables => Box::new(FrogAttackablesSensor),
            Self::IsInWater => Box::new(IsInWaterSensor),
            Self::ArmadilloScareDetected => Box::new(MobSensor::new(
                armadillo::is_scared_by,
                armadillo::can_stay_rolled_up,
                types::DANGER_DETECTED_RECENTLY,
                armadillo::SCARE_CHECK_INTERVAL,
            )),
            Self::BreezeAttackEntitySensor => Box::new(BreezeAttackEntitySensor),
        }
    }

    pub fn create<R: Rng>(self, rng: &mut R) -> SensorEntry {
        SensorEntry::new(self, rng)
    }
}

pub struct SensorEntry {
    ty: SensorType,
    sensor: Box<dyn Sensor>,
    scan_rate: i32,
    time_to_tick: i64,
}

impl SensorEntry {
    pub fn new<R: Rng>(ty: SensorType, rng: &mut R) -> Self {
        let scan_rate = ty.scan_rate();
        Self {
            ty,
            sensor: ty.create_sensor(),
            scan_rate,
            time_to_tick: i64::from(rng.random_range(0..scan_rate)),
        }
    }

    #[must_use]
    pub const fn sensor_type(&self) -> SensorType {
        self.ty
    }

    #[must_use]
    pub fn requires(&self) -> &'static [MemoryModuleId] {
        self.sensor.requires()
    }

    pub fn tick(&mut self, tick: &mut BrainTick<'_>) {
        self.time_to_tick -= 1;
        if self.time_to_tick <= 0 {
            self.time_to_tick = i64::from(self.scan_rate);
            self.sensor.do_tick(tick);
        }
    }
}

/// Entities meeting `body`'s box inflated by the given extents, nearest first.
fn entities_in_inflated_box(
    world: &Arc<World>,
    body: &Entity,
    x: f64,
    y: f64,
    z: f64,
    filter: impl Fn(&Arc<dyn EntityBase>) -> bool,
) -> Vec<Arc<dyn EntityBase>> {
    let bounds = body.bounding_box.load().expand(x, y, z);
    let body_pos = body.pos.load();
    let body_id = body.entity_id;

    let entities = world.entities.load();
    let players = world.players.load();
    // Players are not in world.entities
    let mut found: Vec<(f64, Arc<dyn EntityBase>)> = entities
        .iter()
        .map(Arc::clone)
        .chain(
            players
                .iter()
                .map(|player| Arc::clone(player) as Arc<dyn EntityBase>),
        )
        .filter(|entity| {
            entity.get_entity().entity_id != body_id
                && entity.get_entity().bounding_box.load().intersects(&bounds)
                && filter(entity)
        })
        .map(|entity| {
            (
                entity
                    .get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&body_pos),
                entity,
            )
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, entity)| entity).collect()
}

fn is_current_attack_target(ctx: &VisibilityContext<'_>, target: &dyn EntityBase) -> bool {
    ctx.brain
        .get(types::ATTACK_TARGET)
        .is_some_and(|current| current.get_entity().entity_id == target.get_entity().entity_id)
}

// ignoreInvisibilityTesting drops the range scaling, not line of sight
fn targeting_predicate(
    ctx: &VisibilityContext<'_>,
    target: &dyn EntityBase,
    attackable: bool,
) -> TargetPredicate {
    let predicate = if attackable {
        TargetPredicate::create_attackable()
    } else {
        TargetPredicate::create_non_attackable()
    }
    .set_base_max_distance(ctx.follow_range);
    if is_current_attack_target(ctx, target) {
        predicate.ignore_distance_scaling_factor()
    } else {
        predicate
    }
}

#[must_use]
pub fn is_entity_targetable(ctx: &VisibilityContext<'_>, target: &dyn EntityBase) -> bool {
    targeting_predicate(ctx, target, false).test(ctx.world, Some(ctx.mob), target)
}

#[must_use]
pub fn is_entity_attackable(ctx: &VisibilityContext<'_>, target: &dyn EntityBase) -> bool {
    targeting_predicate(ctx, target, true).test(ctx.world, Some(ctx.mob), target)
}

#[must_use]
pub fn is_entity_attackable_ignoring_line_of_sight(
    ctx: &VisibilityContext<'_>,
    target: &dyn EntityBase,
) -> bool {
    targeting_predicate(ctx, target, true)
        .ignore_visibility()
        .test(ctx.world, Some(ctx.mob), target)
}

pub fn remember_positives<T>(
    invocations: i32,
    predicate: impl Fn(&T) -> bool,
) -> impl FnMut(&T) -> bool {
    let mut positives_left = 0;
    move |value| {
        if predicate(value) {
            positives_left = invocations;
            true
        } else {
            positives_left -= 1;
            positives_left >= 0
        }
    }
}

/// Vanilla `Sensor.wasEntityAttackableLastNTicks`: `isEntityAttackable`, through vanilla's
/// `rememberPositives`, so it stays true for `ticks` calls after the last time it held.
pub fn was_entity_attackable_last_n_ticks(
    ticks: i32,
) -> impl Fn(&BrainTick<'_>, &dyn EntityBase) -> bool + Send + Sync {
    let positives_left = AtomicI32::new(0);
    move |tick: &BrainTick<'_>, target: &dyn EntityBase| {
        if is_entity_attackable(&tick.visibility(), target) {
            positives_left.store(ticks, Ordering::Relaxed);
            true
        } else {
            // `AtomicInteger.decrementAndGet`, which wraps.
            positives_left
                .fetch_sub(1, Ordering::Relaxed)
                .wrapping_sub(1)
                >= 0
        }
    }
}
