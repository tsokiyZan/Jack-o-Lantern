//! Vanilla `FrogAi`, with `ShootTongue`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use pumpkin_data::Block;
use pumpkin_data::entity::{EntityPose, EntityType};
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;
use rand::RngExt;
use uuid::Uuid;

use super::frog::{self, FrogEntity};
use crate::entity::EntityBase;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::long_jump_to_random_pos::default_acceptable_landing_spot;
use crate::entity::ai::brain::behavior::one_shot::trigger_if;
use crate::entity::ai::brain::behavior::utils::{is_breeding, look_at_entity};
use crate::entity::ai::brain::behavior::{
    self, AnimalMakeLove, AnimalPanic, Behavior, CountDownCooldownTicks, Croak, FollowTemptation,
    GateBehavior, LongJumpMidJump, LongJumpToRandomPos, LookAtTargetSink, MoveToTargetSink,
    OrderPolicy, RunningPolicy, Timed,
};
use crate::entity::ai::brain::memory::walk_target::WalkTarget;
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};
use crate::entity::ai::brain::sensing::SensorType;
use crate::entity::ai::brain::{BrainProvider, BrainTick};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::pathfinder::pathfinding_context::PathfindingContext;
use crate::entity::ai::pathfinder::walk_node_evaluator::WalkNodeEvaluator;

const SPEED_MULTIPLIER_WHEN_PANICKING: f32 = 2.0;
const SPEED_MULTIPLIER_WHEN_IDLING: f32 = 1.0;
const SPEED_MULTIPLIER_ON_LAND: f32 = 1.0;
const SPEED_MULTIPLIER_IN_WATER: f32 = 0.75;
/// Vanilla `FrogAi.TIME_BETWEEN_LONG_JUMPS`, both ends inclusive.
const TIME_BETWEEN_LONG_JUMPS: (i32, i32) = (100, 140);
const MAX_LONG_JUMP_HEIGHT: i32 = 2;
const MAX_LONG_JUMP_WIDTH: i32 = 4;
const MAX_JUMP_VELOCITY_MULTIPLIER: f32 = 3.571_428_8;
const SPEED_MULTIPLIER_WHEN_TEMPTED: f32 = 1.25;
/// `LongJumpToPreferredBlock`'s chance to want a block frogs prefer.
const PREFERRED_BLOCKS_CHANCE: f32 = 0.5;

/// Vanilla `FrogAi.initMemories`.
pub fn init_memories(tick: &mut BrainTick<'_>) {
    let (min, max) = TIME_BETWEEN_LONG_JUMPS;
    let cooldown = tick.mob.get_random().random_range(min..=max);
    tick.brain.set(types::LONG_JUMP_COOLDOWN_TICKS, cooldown);
}

fn look_at_players_sometimes() -> Box<dyn behavior::BehaviorControl> {
    Box::new(behavior::set_entity_look_target_sometimes(
        |entity| entity.get_entity().entity_type == &EntityType::PLAYER,
        6.0,
        30,
        60,
    ))
}

fn start_attacking() -> Box<dyn behavior::BehaviorControl> {
    Box::new(behavior::start_attacking(can_attack, |tick| {
        tick.brain.get(types::NEAREST_ATTACKABLE).map(Arc::clone)
    }))
}

fn on_ground(tick: &BrainTick<'_>) -> bool {
    tick.mob.get_entity().on_ground.load(Ordering::Relaxed)
}

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(AnimalPanic::with_damage_types(
                SPEED_MULTIPLIER_WHEN_PANICKING,
                &tag::DamageType::MINECRAFT_PANIC_CAUSES,
            ))),
            Box::new(Timed::new(LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(MoveToTargetSink::default())),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::TEMPTATION_COOLDOWN_TICKS,
            ))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::LONG_JUMP_COOLDOWN_TICKS,
            ))),
        ],
    )
}

fn init_idle_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::Idle,
        vec![
            (0, look_at_players_sometimes()),
            (
                0,
                Box::new(Timed::new(AnimalMakeLove::new(&EntityType::FROG, 1.0, 2))),
            ),
            (
                1,
                Box::new(Timed::new(FollowTemptation::new(|_| {
                    SPEED_MULTIPLIER_WHEN_TEMPTED
                }))),
            ),
            (2, start_attacking()),
            (
                3,
                Box::new(behavior::try_find_land(6, SPEED_MULTIPLIER_WHEN_IDLING)),
            ),
            (
                4,
                Box::new(behavior::run_one_with_conditions(
                    vec![(types::WALK_TARGET.id(), MemoryStatus::ValueAbsent)],
                    vec![
                        (
                            Box::new(behavior::stroll(SPEED_MULTIPLIER_WHEN_IDLING, true)),
                            1,
                        ),
                        (
                            Box::new(behavior::set_walk_target_from_look_target(
                                |_| true,
                                SPEED_MULTIPLIER_WHEN_IDLING,
                                3,
                            )),
                            1,
                        ),
                        (Box::new(Timed::new(Croak::default())), 3),
                        (Box::new(trigger_if("TriggerIf", Vec::new(), on_ground)), 2),
                    ],
                )),
            ),
        ],
        vec![
            (types::LONG_JUMP_MID_JUMP.id(), MemoryStatus::ValueAbsent),
            (types::IS_IN_WATER.id(), MemoryStatus::ValueAbsent),
        ],
    )
}

fn init_swim_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::Swim,
        vec![
            (0, look_at_players_sometimes()),
            (
                1,
                Box::new(Timed::new(FollowTemptation::new(|_| {
                    SPEED_MULTIPLIER_WHEN_TEMPTED
                }))),
            ),
            (2, start_attacking()),
            (3, Box::new(behavior::try_find_land(8, 1.5))),
            (
                5,
                Box::new(GateBehavior::new(
                    "Gate",
                    vec![(types::WALK_TARGET.id(), MemoryStatus::ValueAbsent)],
                    Vec::new(),
                    OrderPolicy::Ordered,
                    RunningPolicy::TryAll,
                    vec![
                        (Box::new(behavior::swim(SPEED_MULTIPLIER_IN_WATER)), 1),
                        (
                            Box::new(behavior::stroll(SPEED_MULTIPLIER_ON_LAND, true)),
                            1,
                        ),
                        (
                            Box::new(behavior::set_walk_target_from_look_target(
                                |_| true,
                                SPEED_MULTIPLIER_ON_LAND,
                                3,
                            )),
                            1,
                        ),
                        (
                            Box::new(trigger_if("TriggerIf", Vec::new(), |tick| {
                                tick.mob.get_entity().is_in_water()
                            })),
                            5,
                        ),
                    ],
                )),
            ),
        ],
        vec![
            (types::LONG_JUMP_MID_JUMP.id(), MemoryStatus::ValueAbsent),
            (types::IS_IN_WATER.id(), MemoryStatus::ValuePresent),
        ],
    )
}

fn init_lay_spawn_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::LaySpawn,
        vec![
            (0, look_at_players_sometimes()),
            (1, start_attacking()),
            (
                2,
                Box::new(behavior::try_find_land_near_liquid(
                    8,
                    SPEED_MULTIPLIER_ON_LAND,
                    &tag::Fluid::MINECRAFT_FROG_TRIES_TO_FIND_LAND_NEAR,
                )),
            ),
            // TODO: FrogspawnBlock, whose 3600 to 12000 tick hatch delay does not fit
            // pumpkin-world's u8 scheduled tick delay; laid frogspawn never hatches.
            (
                3,
                Box::new(behavior::try_lay_spawn_on_fluid_near_land(
                    &Block::FROGSPAWN,
                )),
            ),
            (
                4,
                Box::new(behavior::run_one(vec![
                    (
                        Box::new(behavior::stroll(SPEED_MULTIPLIER_ON_LAND, true)),
                        2,
                    ),
                    (
                        Box::new(behavior::set_walk_target_from_look_target(
                            |_| true,
                            SPEED_MULTIPLIER_ON_LAND,
                            3,
                        )),
                        1,
                    ),
                    (Box::new(Timed::new(Croak::default())), 2),
                    (Box::new(trigger_if("TriggerIf", Vec::new(), on_ground)), 1),
                ])),
            ),
        ],
        vec![
            (types::LONG_JUMP_MID_JUMP.id(), MemoryStatus::ValueAbsent),
            (types::IS_PREGNANT.id(), MemoryStatus::ValuePresent),
        ],
    )
}

fn init_jump_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::LongJump,
        vec![
            (
                0,
                Box::new(Timed::new(LongJumpMidJump::new(
                    TIME_BETWEEN_LONG_JUMPS,
                    Sound::EntityFrogStep,
                ))),
            ),
            (
                1,
                Box::new(Timed::new(LongJumpToRandomPos::to_preferred_block(
                    TIME_BETWEEN_LONG_JUMPS,
                    MAX_LONG_JUMP_HEIGHT,
                    MAX_LONG_JUMP_WIDTH,
                    MAX_JUMP_VELOCITY_MULTIPLIER,
                    |_| Sound::EntityFrogLongJump,
                    &tag::Block::MINECRAFT_FROG_PREFER_JUMP_TO,
                    PREFERRED_BLOCKS_CHANCE,
                    is_acceptable_landing_spot,
                ))),
            ),
        ],
        vec![
            (types::TEMPTING_PLAYER.id(), MemoryStatus::ValueAbsent),
            (types::BREED_TARGET.id(), MemoryStatus::ValueAbsent),
            (
                types::LONG_JUMP_COOLDOWN_TICKS.id(),
                MemoryStatus::ValueAbsent,
            ),
            (types::IS_IN_WATER.id(), MemoryStatus::ValueAbsent),
        ],
    )
}

fn init_tongue_activity() -> ActivityData {
    ActivityData::with_gate_memory(
        Activity::Tongue,
        0,
        vec![
            Box::new(behavior::stop_attacking_if_target_invalid(
                |_, _| false,
                |_, _| {},
                true,
            )),
            Box::new(Timed::new(ShootTongue::new(
                Sound::EntityFrogTongue,
                Sound::EntityFrogEat,
            ))),
        ],
        types::ATTACK_TARGET.id(),
    )
}

/// Vanilla `FrogAi.isAcceptableLandingSpot`.
fn is_acceptable_landing_spot(tick: &BrainTick<'_>, target_pos: &BlockPos) -> bool {
    let world = tick.world;
    let below = target_pos.down();
    let has_fluid = |pos: &BlockPos| world.get_fluid_type(pos).id != Fluid::EMPTY.id;
    if has_fluid(target_pos) || has_fluid(&below) || has_fluid(&target_pos.up()) {
        return false;
    }
    let (block, state) = world.get_block_and_state(target_pos);
    let block_below = world.get_block(&below);
    if block.has_tag(&tag::Block::MINECRAFT_FROG_PREFER_JUMP_TO)
        || block_below.has_tag(&tag::Block::MINECRAFT_FROG_PREFER_JUMP_TO)
    {
        return true;
    }
    let mut context =
        PathfindingContext::new(tick.mob.get_entity().block_pos.load().0, Arc::clone(world));
    let path_type = WalkNodeEvaluator::get_path_type_static(&mut context, target_pos.0);
    let path_type_below = WalkNodeEvaluator::get_path_type_static(&mut context, below.0);
    if path_type == PathType::Trapdoor || state.is_air() && path_type_below == PathType::Trapdoor {
        true
    } else {
        default_acceptable_landing_spot(tick, target_pos)
    }
}

/// Vanilla `FrogAi.canAttack`.
fn can_attack(tick: &BrainTick<'_>) -> bool {
    !is_breeding(tick.brain)
}

/// Vanilla `FrogAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    tick.brain.set_active_activity_to_first_valid(&[
        Activity::Tongue,
        Activity::LaySpawn,
        Activity::LongJump,
        Activity::Swim,
        Activity::Idle,
    ]);
}

/// Vanilla `FrogAi.getTemptations`.
#[must_use]
pub fn get_temptations(stack: &ItemStack) -> bool {
    stack.item.has_tag(&tag::Item::MINECRAFT_FROG_FOOD)
}

pub const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::HurtBy,
    SensorType::FrogAttackables,
    SensorType::FrogTemptations,
    SensorType::IsInWater,
];

pub static FROG_PROVIDER: BrainProvider = BrainProvider {
    memory_types: &[],
    sensor_types: SENSOR_TYPES,
    activities: |_| {
        vec![
            init_core_activity(),
            init_idle_activity(),
            init_swim_activity(),
            init_lay_spawn_activity(),
            init_tongue_activity(),
            init_jump_activity(),
        ]
    },
};

/// Vanilla `ShootTongue.TIME_OUT_DURATION`.
const TIME_OUT_DURATION: i32 = 100;
/// Vanilla `ShootTongue.CATCH_ANIMATION_DURATION`.
const CATCH_ANIMATION_DURATION: i32 = 6;
/// Vanilla `ShootTongue.TONGUE_ANIMATION_DURATION`.
const TONGUE_ANIMATION_DURATION: i32 = 10;
/// Vanilla `ShootTongue.EATING_DISTANCE`.
const EATING_DISTANCE: f32 = 1.75;
/// Vanilla `ShootTongue.EATING_MOVEMENT_FACTOR`.
const EATING_MOVEMENT_FACTOR: f32 = 0.75;
/// Vanilla `ShootTongue.UNREACHABLE_TONGUE_TARGETS_COOLDOWN_DURATION`.
const UNREACHABLE_TONGUE_TARGETS_COOLDOWN_DURATION: i64 = 100;
/// Vanilla `ShootTongue.MAX_UNREACHBLE_TONGUE_TARGETS_IN_MEMORY`.
const MAX_UNREACHABLE_TONGUE_TARGETS_IN_MEMORY: usize = 5;
/// Ticks between `ShootTongue`'s walk target refreshes.
const CALCULATE_PATH_INTERVAL: i32 = 10;
/// The speed `ShootTongue` walks to its target at.
const SPEED_MODIFIER_TO_TARGET: f32 = 2.0;

/// Vanilla `ShootTongue.State`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    MoveToTarget,
    CatchAnimation,
    EatAnimation,
    Done,
}

const SHOOT_TONGUE_CONDITIONS: [(MemoryModuleId, MemoryStatus); 4] = [
    (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
    (types::LOOK_TARGET.id(), MemoryStatus::Registered),
    (types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent),
    (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
];

/// Vanilla `ShootTongue`: walks up to its prey, pulls it in with the tongue and eats it.
struct ShootTongue {
    eat_animation_timer: i32,
    calculate_path_counter: i32,
    tongue_sound: Sound,
    eat_sound: Sound,
    state: State,
}

impl ShootTongue {
    const fn new(tongue_sound: Sound, eat_sound: Sound) -> Self {
        Self {
            eat_animation_timer: 0,
            calculate_path_counter: 0,
            tongue_sound,
            eat_sound,
            state: State::Done,
        }
    }

    fn frog<'a>(tick: &'a BrainTick<'_>) -> Option<&'a FrogEntity> {
        tick.mob.cast_any().downcast_ref::<FrogEntity>()
    }

    fn attack_target(tick: &BrainTick<'_>) -> Option<Arc<dyn EntityBase>> {
        tick.brain.get(types::ATTACK_TARGET).map(Arc::clone)
    }

    fn play_sound(tick: &BrainTick<'_>, sound: Sound) {
        tick.world.play_sound_fine(
            sound,
            SoundCategory::Neutral,
            &tick.mob.get_entity().pos.load(),
            2.0,
            1.0,
        );
    }

    /// Vanilla `ShootTongue.eatEntity`.
    fn eat_entity(&self, tick: &BrainTick<'_>) {
        Self::play_sound(tick, self.eat_sound);
        let Some(target) = Self::frog(tick).and_then(FrogEntity::get_tongue_target) else {
            return;
        };
        if target.get_entity().is_alive() {
            let attacker: &dyn EntityBase = tick.mob;
            tick.mob
                .get_mob_entity()
                .try_attack(attacker, target.as_ref());
            if !behavior::utils::is_alive(target.as_ref()) {
                target.get_entity().remove();
            }
        }
    }

    /// Vanilla `ShootTongue.canPathfindToTarget`.
    fn can_pathfind_to_target(tick: &BrainTick<'_>, target: &dyn EntityBase) -> bool {
        let mob_entity = tick.mob.get_mob_entity();
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .create_path(&mob_entity.living_entity, target.get_entity().pos.load(), 0)
            .is_some_and(|path| path.get_dist_to_target() < EATING_DISTANCE)
    }

    /// Vanilla `ShootTongue.addUnreachableTargetToMemory`.
    fn add_unreachable_target_to_memory(tick: &mut BrainTick<'_>, uuid: Uuid) {
        let mut unreachable_targets = tick
            .brain
            .get(types::UNREACHABLE_TONGUE_TARGETS)
            .cloned()
            .unwrap_or_default();
        let should_add_unreachable_target = !unreachable_targets.contains(&uuid);
        if unreachable_targets.len() == MAX_UNREACHABLE_TONGUE_TARGETS_IN_MEMORY
            && should_add_unreachable_target
        {
            unreachable_targets.remove(0);
        }
        if should_add_unreachable_target {
            unreachable_targets.push(uuid);
        }
        tick.brain.set_with_expiry(
            types::UNREACHABLE_TONGUE_TARGETS,
            unreachable_targets,
            UNREACHABLE_TONGUE_TARGETS_COOLDOWN_DURATION,
        );
    }

    fn walk_to(tick: &mut BrainTick<'_>, target: &dyn EntityBase) {
        tick.brain.set(
            types::WALK_TARGET,
            WalkTarget::from_vec(target.get_entity().pos.load(), SPEED_MODIFIER_TO_TARGET, 0),
        );
    }
}

impl Behavior for ShootTongue {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &SHOOT_TONGUE_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn max_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        let Some(target) = Self::attack_target(tick) else {
            return false;
        };
        let can_pathfind_to_target = Self::can_pathfind_to_target(tick, target.as_ref());
        if !can_pathfind_to_target {
            tick.brain.erase(types::ATTACK_TARGET.id());
            Self::add_unreachable_target_to_memory(tick, target.get_entity().entity_uuid);
        }
        can_pathfind_to_target
            && tick.mob.get_entity().pose.load() != EntityPose::Croaking
            && frog::can_eat(target.as_ref())
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        tick.brain.has_memory_value(types::ATTACK_TARGET.id())
            && self.state != State::Done
            && !tick.brain.has_memory_value(types::IS_PANICKING.id())
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        let Some(target) = Self::attack_target(tick) else {
            return;
        };
        look_at_entity(tick.brain, Arc::clone(&target));
        if let Some(frog) = Self::frog(tick) {
            frog.set_tongue_target(target.as_ref());
        }
        Self::walk_to(tick, target.as_ref());
        self.calculate_path_counter = CALCULATE_PATH_INTERVAL;
        self.state = State::MoveToTarget;
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        tick.brain.erase(types::ATTACK_TARGET.id());
        if let Some(frog) = Self::frog(tick) {
            frog.erase_tongue_target();
        }
        tick.mob.set_pose(EntityPose::Standing);
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(target) = Self::attack_target(tick) else {
            return;
        };
        if let Some(frog) = Self::frog(tick) {
            frog.set_tongue_target(target.as_ref());
        }
        match self.state {
            State::MoveToTarget => {
                let body_pos = tick.mob.get_entity().pos.load();
                let target_entity = target.get_entity();
                let target_pos = target_entity.pos.load();
                let distance = target_pos.squared_distance_to_vec(&body_pos).sqrt() as f32;
                if distance < EATING_DISTANCE {
                    Self::play_sound(tick, self.tongue_sound);
                    tick.mob.set_pose(EntityPose::UsingTongue);
                    target_entity.velocity.store(
                        (body_pos - target_pos).normalize() * f64::from(EATING_MOVEMENT_FACTOR),
                    );
                    self.eat_animation_timer = 0;
                    self.state = State::CatchAnimation;
                } else if self.calculate_path_counter <= 0 {
                    Self::walk_to(tick, target.as_ref());
                    self.calculate_path_counter = CALCULATE_PATH_INTERVAL;
                } else {
                    self.calculate_path_counter -= 1;
                }
            }
            State::CatchAnimation => {
                let timer = self.eat_animation_timer;
                self.eat_animation_timer += 1;
                if timer >= CATCH_ANIMATION_DURATION {
                    self.state = State::EatAnimation;
                    self.eat_entity(tick);
                }
            }
            State::EatAnimation => {
                if self.eat_animation_timer >= TONGUE_ANIMATION_DURATION {
                    self.state = State::Done;
                } else {
                    self.eat_animation_timer += 1;
                }
            }
            State::Done => {}
        }
    }

    fn debug_name(&self) -> &'static str {
        "ShootTongue"
    }
}
