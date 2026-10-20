//! Vanilla `AllayAi`.

use std::sync::Arc;

use pumpkin_data::Block;
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::GameMode;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::EntityBase;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::{
    self, AnimalPanic, CountDownCooldownTicks, DoNothing, GoAndGiveItemsToTarget, Swim, Timed,
};
use crate::entity::ai::brain::memory::position_tracker::{
    BlockPosTracker, EntityTracker, PositionTracker,
};
use crate::entity::ai::brain::memory::{GlobalPos, MemoryModuleId, types};
use crate::entity::ai::brain::sensing::SensorType;
use crate::entity::ai::brain::{BrainProvider, BrainTick};
use crate::entity::passive::allay::{AllayEntity, MAX_NOTEBLOCK_DISTANCE, THROW_SOUND_PITCHES};
use crate::entity::player::Player;

const SPEED_MULTIPLIER_WHEN_IDLING: f32 = 1.0;
const SPEED_MULTIPLIER_WHEN_FOLLOWING_DEPOSIT_TARGET: f32 = 2.25;
const SPEED_MULTIPLIER_WHEN_RETRIEVING_ITEM: f32 = 1.75;
const SPEED_MULTIPLIER_WHEN_PANICKING: f32 = 2.5;
const CLOSE_ENOUGH_TO_TARGET: i32 = 4;
const TOO_FAR_FROM_TARGET: i32 = 16;
const MAX_LOOK_DISTANCE: f32 = 6.0;
const MIN_WAIT_DURATION: i32 = 30;
const MAX_WAIT_DURATION: i32 = 60;
const TIME_TO_FORGET_NOTEBLOCK: i32 = 600;
const DISTANCE_TO_WANTED_ITEM: i32 = 32;
const GIVE_ITEM_TIMEOUT_DURATION: i32 = 20;
/// Vanilla `AllayAi.THROW_VELOCITY`, a `Vec3` of floats.
const THROW_VELOCITY: (f32, f32, f32) = (0.2, 0.3, 0.2);
const ITEM_PICKUP_COOLDOWN_DURATION: i32 = 60;
/// The hand height `AllayAi.throwItem` passes to `BehaviorUtils.throwItem`.
const THROW_HAND_Y_DISTANCE_FROM_EYE: f32 = 0.2;
/// Vanilla `ServerPlayer.closerThan(allay, 64.0)` in `getLikedPlayer`.
const MAX_LIKED_PLAYER_DISTANCE: f64 = 64.0;

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(Swim::new(0.8))),
            Box::new(Timed::new(AnimalPanic::new(
                SPEED_MULTIPLIER_WHEN_PANICKING,
            ))),
            Box::new(Timed::new(behavior::LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(behavior::MoveToTargetSink::default())),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::LIKED_NOTEBLOCK_COOLDOWN_TICKS,
            ))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::ITEM_PICKUP_COOLDOWN_TICKS,
            ))),
        ],
    )
}

fn init_idle_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Idle,
        0,
        vec![
            Box::new(behavior::go_to_wanted_item(
                |_| true,
                SPEED_MULTIPLIER_WHEN_RETRIEVING_ITEM,
                true,
                DISTANCE_TO_WANTED_ITEM,
            )),
            Box::new(GoAndGiveItemsToTarget::new(
                get_item_deposit_position,
                SPEED_MULTIPLIER_WHEN_FOLLOWING_DEPOSIT_TARGET,
                GIVE_ITEM_TIMEOUT_DURATION,
                throw_item,
                types::ITEM_PICKUP_COOLDOWN_TICKS,
                ITEM_PICKUP_COOLDOWN_DURATION,
                has_item_to_throw,
            )),
            Box::new(behavior::stay_close_to_target(
                get_item_deposit_position,
                |tick| !has_wanted_item(tick),
                CLOSE_ENOUGH_TO_TARGET,
                TOO_FAR_FROM_TARGET,
                SPEED_MULTIPLIER_WHEN_FOLLOWING_DEPOSIT_TARGET,
            )),
            Box::new(behavior::set_entity_look_target_sometimes(
                |_| true,
                MAX_LOOK_DISTANCE,
                MIN_WAIT_DURATION,
                MAX_WAIT_DURATION,
            )),
            Box::new(behavior::run_one(vec![
                (Box::new(behavior::fly(SPEED_MULTIPLIER_WHEN_IDLING)), 2),
                (
                    Box::new(behavior::set_walk_target_from_look_target(
                        |_| true,
                        SPEED_MULTIPLIER_WHEN_IDLING,
                        3,
                    )),
                    2,
                ),
                (
                    Box::new(DoNothing::new(MIN_WAIT_DURATION, MAX_WAIT_DURATION)),
                    1,
                ),
            ])),
        ],
    )
}

fn has_item_to_throw(tick: &BrainTick<'_>) -> bool {
    tick.mob
        .cast_any()
        .downcast_ref::<AllayEntity>()
        .is_some_and(|allay| !allay.inventory.is_empty())
}

/// Vanilla `AllayAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    tick.brain
        .set_active_activity_to_first_valid(&[Activity::Idle]);
}

/// Vanilla `AllayAi.hearNoteblock`.
pub fn hear_noteblock(tick: &mut BrainTick<'_>, pos: BlockPos) {
    let Some(global_pos) = behavior::utils::global_pos_in(tick.world, pos) else {
        return;
    };
    match tick.brain.get(types::LIKED_NOTEBLOCK_POSITION).copied() {
        None => {
            tick.brain.set(types::LIKED_NOTEBLOCK_POSITION, global_pos);
            tick.brain.set(
                types::LIKED_NOTEBLOCK_COOLDOWN_TICKS,
                TIME_TO_FORGET_NOTEBLOCK,
            );
        }
        Some(liked_noteblock_pos) if liked_noteblock_pos == global_pos => {
            tick.brain.set(
                types::LIKED_NOTEBLOCK_COOLDOWN_TICKS,
                TIME_TO_FORGET_NOTEBLOCK,
            );
        }
        Some(_) => {}
    }
}

/// Vanilla `AllayAi.getItemDepositPosition`: the liked note block while it still counts,
/// otherwise the liked player.
fn get_item_deposit_position(tick: &mut BrainTick<'_>) -> Option<Arc<dyn PositionTracker>> {
    if let Some(position) = tick.brain.get(types::LIKED_NOTEBLOCK_POSITION).copied() {
        if should_deposit_items_at_liked_noteblock(tick, position) {
            return Some(Arc::new(BlockPosTracker::new(position.pos.up())));
        }
        tick.brain.erase(types::LIKED_NOTEBLOCK_POSITION.id());
    }
    get_liked_player_position_tracker(tick)
}

fn has_wanted_item(tick: &BrainTick<'_>) -> bool {
    tick.brain
        .has_memory_value(types::NEAREST_VISIBLE_WANTED_ITEM.id())
}

fn should_deposit_items_at_liked_noteblock(
    tick: &BrainTick<'_>,
    liked_noteblock_pos: GlobalPos,
) -> bool {
    liked_noteblock_pos.is_close_enough(
        &tick.world.dimension,
        tick.mob.get_entity().block_pos.load(),
        MAX_NOTEBLOCK_DISTANCE,
    ) && tick.world.get_block(&liked_noteblock_pos.pos) == &Block::NOTE_BLOCK
        && tick
            .brain
            .has_memory_value(types::LIKED_NOTEBLOCK_COOLDOWN_TICKS.id())
}

fn get_liked_player_position_tracker(tick: &BrainTick<'_>) -> Option<Arc<dyn PositionTracker>> {
    get_liked_player(tick).map(|player| {
        Arc::new(EntityTracker::new(player as Arc<dyn EntityBase>, true))
            as Arc<dyn PositionTracker>
    })
}

/// Vanilla `AllayAi.getLikedPlayer`: the liked player, while in this world, not a
/// spectator and within 64 blocks.
fn get_liked_player(tick: &BrainTick<'_>) -> Option<Arc<Player>> {
    let liked_player = *tick.brain.get(types::LIKED_PLAYER)?;
    let player = tick.world.get_player_by_uuid(liked_player)?;
    let game_mode = player.gamemode.load();
    let max_distance_squared = MAX_LIKED_PLAYER_DISTANCE * MAX_LIKED_PLAYER_DISTANCE;
    (matches!(
        game_mode,
        GameMode::Survival | GameMode::Adventure | GameMode::Creative
    ) && player
        .get_entity()
        .pos
        .load()
        .squared_distance_to_vec(&tick.mob.get_entity().pos.load())
        < max_distance_squared)
        .then_some(player)
}

/// Vanilla `AllayAi.throwItem`.
fn throw_item(tick: &mut BrainTick<'_>, target_pos: Vector3<f64>) {
    let Some(allay) = tick.mob.cast_any().downcast_ref::<AllayEntity>() else {
        return;
    };
    let item = allay.inventory.remove_item(1);
    if item.is_empty() {
        return;
    }
    let entity = allay.get_entity();
    behavior::utils::throw_item(
        entity,
        item,
        target_pos + Vector3::new(0.0, 1.0, 0.0),
        Vector3::new(
            f64::from(THROW_VELOCITY.0),
            f64::from(THROW_VELOCITY.1),
            f64::from(THROW_VELOCITY.2),
        ),
        THROW_HAND_Y_DISTANCE_FROM_EYE,
    );
    // TODO: `CriteriaTriggers.ALLAY_DROP_ITEM_ON_BLOCK` for the liked player.
    // `level.getRandom()` for both the roll and `Util.getRandom` over the pitches.
    let pitch = (tick.time % 7 == 0)
        .then(|| {
            let mut random = tick
                .world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (random.next_f64() < 0.9).then(|| {
                THROW_SOUND_PITCHES[random.next_bounded_i32(THROW_SOUND_PITCHES.len() as i32) as usize]
            })
        })
        .flatten();
    if let Some(pitch) = pitch {
        tick.world.play_sound_fine(
            Sound::EntityAllayItemThrown,
            SoundCategory::Neutral,
            &entity.pos.load(),
            1.0,
            pitch,
        );
    }
}

/// Vanilla `Allay.BRAIN_PROVIDER`'s memories.
const MEMORY_TYPES: &[MemoryModuleId] = &[
    types::LIKED_PLAYER.id(),
    types::LIKED_NOTEBLOCK_POSITION.id(),
    types::LIKED_NOTEBLOCK_COOLDOWN_TICKS.id(),
];

/// Vanilla `Allay.BRAIN_PROVIDER`'s sensors.
const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::NearestPlayers,
    SensorType::HurtBy,
    SensorType::NearestItems,
];

pub static ALLAY_PROVIDER: BrainProvider = BrainProvider {
    memory_types: MEMORY_TYPES,
    sensor_types: SENSOR_TYPES,
    activities: |_| vec![init_core_activity(), init_idle_activity()],
};
