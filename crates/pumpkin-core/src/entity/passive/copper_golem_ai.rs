//! Vanilla `CopperGolemAi`.

use std::sync::Arc;

use pumpkin_data::entity::EntityType;
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockState};
use pumpkin_inventory::double::DoubleInventory;

use crate::block::blocks::chests;
use crate::block::entities::chest::ChestBlockEntity;
use crate::block::entities::trapped_chest::TrappedChestBlockEntity;
use crate::entity::EntityBase;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::transport_items_between_containers::{
    ContainerInteractionState, OnTargetReachedInteraction, TARGET_INTERACTION_TIME,
    TransportItemTarget,
};
use crate::entity::ai::brain::behavior::{
    self, AnimalPanic, CountDownCooldownTicks, DoNothing, Timed, TransportItemsBetweenContainers,
};
use crate::entity::ai::brain::memory::{MemoryStatus, types};
use crate::entity::ai::brain::sensing::SensorType;
use crate::entity::ai::brain::{BrainProvider, BrainTick};
use crate::entity::passive::copper_golem::{CopperGolemEntity, CopperGolemState};
use crate::world::World;

const SPEED_MULTIPLIER_WHEN_PANICKING: f32 = 1.5;
const SPEED_MULTIPLIER_WHEN_IDLING: f32 = 1.0;
const TRANSPORT_ITEM_HORIZONTAL_SEARCH_RADIUS: i32 = 32;
const TRANSPORT_ITEM_VERTICAL_SEARCH_RADIUS: i32 = 8;
const TICK_TO_START_ON_REACHED_INTERACTION: i32 = 1;
const TICK_TO_PLAY_ON_REACHED_SOUND: i32 = 9;
/// `SetEntityLookTargetSometimes.create(EntityTypes.PLAYER, 6.0F, UniformInt.of(40, 80))`.
const LOOK_AT_PLAYER_MAX_DIST: f32 = 6.0;
const LOOK_AT_PLAYER_MIN_INTERVAL: i32 = 40;
const LOOK_AT_PLAYER_MAX_INTERVAL: i32 = 80;
/// `RandomStroll.stroll(1.0F, 2, 2)`.
const STROLL_MAX_HORIZONTAL_DISTANCE: i32 = 2;
const STROLL_MAX_VERTICAL_DISTANCE: i32 = 2;
/// `new DoNothing(30, 60)`.
const IDLE_MIN_DURATION: i32 = 30;
const IDLE_MAX_DURATION: i32 = 60;

/// Vanilla `TRANSPORT_ITEM_SOURCE_BLOCK`.
fn transport_item_source_block(state: &BlockState) -> bool {
    Block::from_state_id(state.id).has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
}

/// Vanilla `TRANSPORT_ITEM_DESTINATION_BLOCK`.
fn transport_item_destination_block(state: &BlockState) -> bool {
    let block = Block::from_state_id(state.id);
    block == &Block::CHEST || block == &Block::TRAPPED_CHEST
}

/// Vanilla `CopperGolemAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    tick.brain
        .set_active_activity_to_first_valid(&[Activity::Idle]);
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
            Box::new(behavior::interact_with_door()),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::GAZE_COOLDOWN_TICKS,
            ))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::TRANSPORT_ITEMS_COOLDOWN_TICKS,
            ))),
        ],
    )
}

fn init_idle_activity() -> ActivityData {
    ActivityData::with_pairs(
        Activity::Idle,
        vec![
            (
                0,
                Box::new(Timed::new(TransportItemsBetweenContainers::new(
                    SPEED_MULTIPLIER_WHEN_IDLING,
                    transport_item_source_block,
                    transport_item_destination_block,
                    TRANSPORT_ITEM_HORIZONTAL_SEARCH_RADIUS,
                    TRANSPORT_ITEM_VERTICAL_SEARCH_RADIUS,
                    get_target_reached_interactions(),
                    on_travelling,
                    should_queue_for_target,
                ))),
            ),
            (
                1,
                Box::new(behavior::set_entity_look_target_sometimes(
                    |entity: &Arc<dyn EntityBase>| {
                        entity.get_entity().entity_type == &EntityType::PLAYER
                    },
                    LOOK_AT_PLAYER_MAX_DIST,
                    LOOK_AT_PLAYER_MIN_INTERVAL,
                    LOOK_AT_PLAYER_MAX_INTERVAL,
                )),
            ),
            (
                2,
                Box::new(behavior::run_one_with_conditions(
                    vec![
                        (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
                        (
                            types::TRANSPORT_ITEMS_COOLDOWN_TICKS.id(),
                            MemoryStatus::ValuePresent,
                        ),
                    ],
                    vec![
                        (
                            Box::new(behavior::stroll_with_range(
                                SPEED_MULTIPLIER_WHEN_IDLING,
                                STROLL_MAX_HORIZONTAL_DISTANCE,
                                STROLL_MAX_VERTICAL_DISTANCE,
                                true,
                            )),
                            1,
                        ),
                        (
                            Box::new(DoNothing::new(IDLE_MIN_DURATION, IDLE_MAX_DURATION)),
                            1,
                        ),
                    ],
                )),
            ),
        ],
    )
}

/// Vanilla `getTargetReachedInteractions`, indexed by `ContainerInteractionState`.
fn get_target_reached_interactions() -> [Option<OnTargetReachedInteraction>; 4] {
    let mut interactions: [Option<OnTargetReachedInteraction>; 4] = [None, None, None, None];
    interactions[ContainerInteractionState::PickupItem as usize] =
        Some(on_reached_target_interaction(
            CopperGolemState::GettingItem,
            Some(Sound::EntityCopperGolemNoItemGet),
        ));
    interactions[ContainerInteractionState::PickupNoItem as usize] =
        Some(on_reached_target_interaction(
            CopperGolemState::GettingNoItem,
            Some(Sound::EntityCopperGolemNoItemNoGet),
        ));
    interactions[ContainerInteractionState::PlaceItem as usize] =
        Some(on_reached_target_interaction(
            CopperGolemState::DroppingItem,
            Some(Sound::EntityCopperGolemItemDrop),
        ));
    interactions[ContainerInteractionState::PlaceNoItem as usize] =
        Some(on_reached_target_interaction(
            CopperGolemState::DroppingNoItem,
            Some(Sound::EntityCopperGolemItemNoDrop),
        ));
    interactions
}

fn on_reached_target_interaction(
    state: CopperGolemState,
    sound: Option<Sound>,
) -> OnTargetReachedInteraction {
    Box::new(move |tick, target, ticks_since_reaching_target| {
        let Some(copper_golem) = tick.mob.cast_any().downcast_ref::<CopperGolemEntity>() else {
            return;
        };
        if ticks_since_reaching_target == TICK_TO_START_ON_REACHED_INTERACTION {
            target.container.on_open();
            copper_golem.set_opened_chest_pos(target.pos);
            copper_golem.set_state(state);
        }
        if ticks_since_reaching_target == TICK_TO_PLAY_ON_REACHED_SOUND
            && let Some(sound) = sound
        {
            copper_golem.play_sound_at_self(sound, 1.0, 1.0);
        }
        if ticks_since_reaching_target == TARGET_INTERACTION_TIME {
            if container_has_user_open(tick.world, target, copper_golem) {
                target.container.on_close();
            }
            copper_golem.clear_opened_chest_pos();
        }
    })
}

/// `container.getEntitiesWithContainerOpen().contains(body)`. A double chest's
/// `CompoundContainer` keeps the default empty list, so only a single chest closes here; the
/// recheck closes a double chest once the golem lets go of it.
fn container_has_user_open(
    world: &World,
    target: &TransportItemTarget,
    copper_golem: &CopperGolemEntity,
) -> bool {
    if target.container.as_any().is::<DoubleInventory>() {
        return false;
    }
    let entity_id = copper_golem.get_entity().entity_id;
    chests::get_entities_with_container_open(world, &target.pos)
        .iter()
        .any(|user| user.get_entity().entity_id == entity_id)
}

/// Vanilla `onTravelling`.
fn on_travelling(tick: &mut BrainTick<'_>) {
    if let Some(copper_golem) = tick.mob.cast_any().downcast_ref::<CopperGolemEntity>() {
        copper_golem.clear_opened_chest_pos();
        copper_golem.set_state(CopperGolemState::Idle);
    }
}

/// Vanilla `shouldQueueForTarget`.
fn should_queue_for_target(world: &World, target: &TransportItemTarget) -> bool {
    let any = target.block_entity.as_any();
    (any.is::<ChestBlockEntity>() || any.is::<TrappedChestBlockEntity>())
        && !chests::get_entities_with_container_open(world, &target.pos).is_empty()
}

/// Vanilla `CopperGolem.BRAIN_PROVIDER`'s sensors.
const SENSOR_TYPES: &[SensorType] = &[SensorType::NearestLivingEntities, SensorType::HurtBy];

pub static COPPER_GOLEM_PROVIDER: BrainProvider = BrainProvider {
    memory_types: &[],
    sensor_types: SENSOR_TYPES,
    activities: |_| vec![init_core_activity(), init_idle_activity()],
};
