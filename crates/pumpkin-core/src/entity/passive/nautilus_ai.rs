//! Vanilla `NautilusAi`.

use std::sync::Arc;

use pumpkin_data::entity::EntityType;
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use rand::RngExt;

use crate::entity::EntityBase;
use crate::entity::ageable::AgeableMob;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::utils::{get_living_entity_from_uuid_memory, is_breeding};
use crate::entity::ai::brain::behavior::{
    self, AnimalMakeLove, AnimalPanic, BehaviorControl, ChargeAttack, CountDownCooldownTicks,
    FollowTemptation, GateBehavior, OrderPolicy, RunningPolicy, Timed,
};
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};
use crate::entity::ai::brain::sensing::{SensorType, is_entity_attackable_ignoring_line_of_sight};
use crate::entity::ai::brain::{BrainProvider, BrainTick};
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::mob::piglin_ai::sample;
use crate::entity::passive::tamable::TamableAnimal;

const SPEED_MULTIPLIER_WHEN_IDLING_IN_WATER: f32 = 1.0;
const SPEED_MULTIPLIER_WHEN_TEMPTED: f32 = 1.3;
const SPEED_MULTIPLIER_WHEN_MAKING_LOVE: f32 = 0.4;
const SPEED_MULTIPLIER_WHEN_PANICKING: f32 = 1.6;
const TIME_BETWEEN_NON_PLAYER_ATTACKS: (i32, i32) = (2400, 3600);
const SPEED_WHEN_ATTACKING: f32 = 0.6;
pub const ATTACK_KNOCKBACK_FORCE: f32 = 2.0;
const ANGER_DURATION: i64 = 400;
pub const TIME_BETWEEN_ATTACKS: i32 = 80;
pub const MAX_CHARGE_DISTANCE: f64 = 12.0;
pub const MAX_TARGET_DETECTION_DISTANCE: f64 = 11.0;
/// The `closeEnoughDistance` of the nautiluses' `FollowTemptation`, as babies and adults.
const BABY_TEMPTED_CLOSE_ENOUGH_DIST: f64 = 2.5;
const ADULT_TEMPTED_CLOSE_ENOUGH_DIST: f64 = 3.5;
/// The weights of the idle `GateBehavior`'s swim and look-target walks.
const SWIM_WEIGHT: i32 = 2;
const WALK_TO_LOOK_TARGET_WEIGHT: i32 = 3;
/// The `closeEnoughDist` of `SetWalkTargetFromLookTarget.create(1.0F, 3)`.
const LOOK_TARGET_CLOSE_ENOUGH_DIST: i32 = 3;
/// The odds `findNearestValidAttackTarget` skips a hostile once its cooldown is over.
const SKIP_NON_PLAYER_ATTACK_CHANCE: f32 = 0.5;

/// Vanilla `NautilusAi.ATTACK_TARGET_CONDITIONS`: armor stands only while mobs may
/// grief, and only inside the world border.
#[must_use]
pub fn attack_target_conditions() -> TargetPredicate {
    let mut conditions = TargetPredicate::create_attackable();
    conditions.set_predicate(|target, world| {
        (world.level_info.load().game_rules.mob_griefing
            || target.entity.entity_type != &EntityType::ARMOR_STAND)
            && world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_box(&target.entity.bounding_box.load())
    });
    conditions
}

/// Vanilla `NautilusAi.initMemories`.
pub fn init_memories(tick: &mut BrainTick<'_>) {
    let cooldown = sample(TIME_BETWEEN_NON_PLAYER_ATTACKS, &mut tick.mob.get_random());
    tick.brain.set(types::ATTACK_TARGET_COOLDOWN, cooldown);
}

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(AnimalPanic::new(
                SPEED_MULTIPLIER_WHEN_PANICKING,
            ))),
            Box::new(Timed::new(behavior::LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(behavior::MoveToTargetSink::default())),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::TEMPTATION_COOLDOWN_TICKS,
            ))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::CHARGE_COOLDOWN_TICKS,
            ))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::ATTACK_TARGET_COOLDOWN,
            ))),
        ],
    )
}

/// The `closeEnoughDistance` both nautiluses give `FollowTemptation`.
#[must_use]
pub fn tempted_close_enough_distance(tick: &BrainTick<'_>) -> f64 {
    if tick.mob.as_ageable().is_some_and(AgeableMob::is_baby) {
        BABY_TEMPTED_CLOSE_ENOUGH_DIST
    } else {
        ADULT_TEMPTED_CLOSE_ENOUGH_DIST
    }
}

/// The idle `GateBehavior` both nautiluses share: swim about, or towards what they look at.
#[must_use]
pub fn idle_movement() -> Box<dyn BehaviorControl> {
    Box::new(GateBehavior::new(
        "Gate",
        vec![(types::WALK_TARGET.id(), MemoryStatus::ValueAbsent)],
        Vec::new(),
        OrderPolicy::Ordered,
        RunningPolicy::TryAll,
        vec![
            (
                Box::new(behavior::swim(SPEED_MULTIPLIER_WHEN_IDLING_IN_WATER)),
                SWIM_WEIGHT,
            ),
            (
                Box::new(behavior::set_walk_target_from_look_target(
                    |_| true,
                    SPEED_MULTIPLIER_WHEN_IDLING_IN_WATER,
                    LOOK_TARGET_CLOSE_ENOUGH_DIST,
                )),
                WALK_TO_LOOK_TARGET_WEIGHT,
            ),
        ],
    ))
}

/// `StartAttacking.create(NautilusAi::findNearestValidAttackTarget)`.
#[must_use]
pub fn start_attacking() -> Box<dyn BehaviorControl> {
    Box::new(behavior::start_attacking_with_memory_writes(
        |_| true,
        find_nearest_valid_attack_target,
    ))
}

fn init_idle_activity() -> ActivityData {
    ActivityData::with_pairs(
        Activity::Idle,
        vec![
            (
                1,
                Box::new(Timed::new(AnimalMakeLove::new(
                    &EntityType::NAUTILUS,
                    SPEED_MULTIPLIER_WHEN_MAKING_LOVE,
                    2,
                ))),
            ),
            (
                2,
                Box::new(Timed::new(
                    FollowTemptation::new(|_| SPEED_MULTIPLIER_WHEN_TEMPTED)
                        .with_close_enough_distance_fn(tempted_close_enough_distance),
                )),
            ),
            (3, start_attacking()),
            (4, idle_movement()),
        ],
    )
}

/// The memory conditions of both nautiluses' fight activity.
#[must_use]
pub fn fight_conditions() -> Vec<(MemoryModuleId, MemoryStatus)> {
    vec![
        (types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent),
        (types::TEMPTING_PLAYER.id(), MemoryStatus::ValueAbsent),
        (types::BREED_TARGET.id(), MemoryStatus::ValueAbsent),
        (types::CHARGE_COOLDOWN_TICKS.id(), MemoryStatus::ValueAbsent),
    ]
}

fn init_fight_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::Fight,
        vec![(
            0,
            Box::new(Timed::new(ChargeAttack::new(
                TIME_BETWEEN_ATTACKS,
                attack_target_conditions(),
                SPEED_WHEN_ATTACKING,
                ATTACK_KNOCKBACK_FORCE,
                MAX_CHARGE_DISTANCE,
                MAX_TARGET_DETECTION_DISTANCE,
                Sound::EntityNautilusDash,
            ))),
        )],
        fight_conditions(),
    )
}

/// Vanilla `NautilusAi.findNearestValidAttackTarget`: whoever it is angry at, else now and
/// then a hostile swimming nearby.
fn find_nearest_valid_attack_target(tick: &mut BrainTick<'_>) -> Option<Arc<dyn EntityBase>> {
    let body = tick.mob;
    if is_breeding(tick.brain)
        || !body.get_entity().is_in_water()
        || body.as_ageable().is_some_and(AgeableMob::is_baby)
        || body.as_tamable().is_some_and(TamableAnimal::is_tame)
    {
        return None;
    }
    let angry_at = get_living_entity_from_uuid_memory(tick.brain, tick.world, types::ANGRY_AT)
        .filter(|entity| {
            entity.get_entity().is_in_water()
                && is_entity_attackable_ignoring_line_of_sight(&tick.visibility(), entity.as_ref())
        });
    if angry_at.is_some() {
        return angry_at;
    }
    if tick
        .brain
        .has_memory_value(types::ATTACK_TARGET_COOLDOWN.id())
    {
        return None;
    }
    let mut random = body.get_random();
    tick.brain.set(
        types::ATTACK_TARGET_COOLDOWN,
        sample(TIME_BETWEEN_NON_PLAYER_ATTACKS, &mut random),
    );
    if random.random::<f32>() < SKIP_NON_PLAYER_ATTACK_CHANCE {
        return None;
    }
    let visibility = tick.visibility();
    tick.brain
        .get(types::NEAREST_VISIBLE_LIVING_ENTITIES)?
        .find_closest(&visibility, |entity| is_hostile_target(entity.as_ref()))
}

/// Vanilla `NautilusAi.setAngerTarget`.
pub fn set_anger_target(tick: &mut BrainTick<'_>, target: &dyn EntityBase) {
    if is_entity_attackable_ignoring_line_of_sight(&tick.visibility(), target) {
        tick.brain.erase(types::CANT_REACH_WALK_TARGET_SINCE.id());
        tick.brain.set_with_expiry(
            types::ANGRY_AT,
            target.get_entity().entity_uuid,
            ANGER_DURATION,
        );
    }
}

/// Vanilla `NautilusAi.isHostileTarget`.
fn is_hostile_target(mob: &dyn EntityBase) -> bool {
    let entity = mob.get_entity();
    entity.is_in_water()
        && entity
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_NAUTILUS_HOSTILES)
}

/// Vanilla `NautilusAi.updateActivity`, which `ZombieNautilusAi.updateActivity` repeats.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    tick.brain
        .set_active_activity_to_first_valid(&[Activity::Fight, Activity::Idle]);
}

/// Vanilla `NautilusAi.getTemptations`.
#[must_use]
pub fn get_temptations(stack: &ItemStack) -> bool {
    stack.item.has_tag(&tag::Item::MINECRAFT_NAUTILUS_FOOD)
}

/// Vanilla `Nautilus.BRAIN_PROVIDER`'s and `ZombieNautilus.BRAIN_PROVIDER`'s memories.
pub const MEMORY_TYPES: &[MemoryModuleId] =
    &[types::ANGRY_AT.id(), types::ATTACK_TARGET_COOLDOWN.id()];

/// Vanilla `Nautilus.BRAIN_PROVIDER`'s and `ZombieNautilus.BRAIN_PROVIDER`'s sensors.
pub const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::NearestAdult,
    SensorType::NearestPlayers,
    SensorType::HurtBy,
    SensorType::NautilusTemptations,
];

pub static NAUTILUS_PROVIDER: BrainProvider = BrainProvider {
    memory_types: MEMORY_TYPES,
    sensor_types: SENSOR_TYPES,
    activities: |_| {
        vec![
            init_core_activity(),
            init_idle_activity(),
            init_fight_activity(),
        ]
    },
};
