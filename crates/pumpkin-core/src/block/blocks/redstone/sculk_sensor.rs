use std::sync::Arc;

use crate::block::blocks::note::get_pitch_from_note;
use crate::block::entities::sculk_sensor::SculkSensorBlockEntity;
use crate::block::{
    BlockBehaviour, BlockMetadata, EmitsRedstonePowerArgs, GetComparatorOutputArgs,
    GetRedstonePowerArgs, OnEntityStepArgs, OnPlaceArgs, OnScheduledTickArgs, OnStateReplacedArgs,
    PathComputationType, PlacedArgs,
};
use crate::entity::EntityBase;
use crate::world::World;
use crate::world::game_event::GameEventContext;
use crate::world::game_event::vibrations::{self, get_resonance_event_by_frequency};
use pumpkin_data::block_properties::{
    CalibratedSculkSensorLikeProperties, HorizontalFacing, SculkSensorLikeProperties,
    SculkSensorPhase,
};
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection, BlockId, BlockState, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

/// Vanilla `SculkSensorBlock.ACTIVE_TICKS`.
pub const ACTIVE_TICKS: u8 = 30;
/// Vanilla `CalibratedSculkSensorBlock.getActiveTicks`.
const CALIBRATED_ACTIVE_TICKS: u8 = 10;
/// Vanilla `SculkSensorBlock.COOLDOWN_TICKS`.
pub const COOLDOWN_TICKS: u8 = 10;
/// The notes of vanilla `SculkSensorBlock.RESONANCE_PITCH_BEND`, by frequency.
const RESONANCE_TONE_MAP: [u16; 16] = [0, 0, 2, 4, 6, 7, 9, 10, 12, 14, 15, 18, 19, 21, 22, 24];

pub struct SculkSensorBlock;

impl BlockMetadata for SculkSensorBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::SCULK_SENSOR, BlockId::CALIBRATED_SCULK_SENSOR].into()
    }
}

const fn horizontal_facing_to_dir(facing: HorizontalFacing) -> BlockDirection {
    match facing {
        HorizontalFacing::North => BlockDirection::North,
        HorizontalFacing::South => BlockDirection::South,
        HorizontalFacing::West => BlockDirection::West,
        HorizontalFacing::East => BlockDirection::East,
    }
}

/// The `FACING` of a calibrated sculk sensor, `None` for any other state.
#[must_use]
pub fn calibrated_facing(state: &BlockState) -> Option<BlockDirection> {
    (Block::from_state_id(state.id) == &Block::CALIBRATED_SCULK_SENSOR).then(|| {
        horizontal_facing_to_dir(
            CalibratedSculkSensorLikeProperties::from_state_id(state.id).facing,
        )
    })
}

/// A sensor's `PHASE`, `POWER` and `WATERLOGGED`, which both variants carry under different
/// property types.
#[derive(Clone, Copy)]
struct SensorProperties {
    phase: SculkSensorPhase,
    power: u8,
    waterlogged: bool,
}

impl SensorProperties {
    fn of(block: &Block, state_id: BlockStateId) -> Self {
        if block == &Block::CALIBRATED_SCULK_SENSOR {
            let props = CalibratedSculkSensorLikeProperties::from_state_id(state_id);
            Self {
                phase: props.sculk_sensor_phase,
                power: props.power,
                waterlogged: props.waterlogged,
            }
        } else {
            let props = SculkSensorLikeProperties::from_state_id(state_id);
            Self {
                phase: props.sculk_sensor_phase,
                power: props.power,
                waterlogged: props.waterlogged,
            }
        }
    }

    /// `state.setValue(PHASE, phase).setValue(POWER, power)`.
    fn with(
        block: &Block,
        state_id: BlockStateId,
        phase: SculkSensorPhase,
        power: u8,
    ) -> BlockStateId {
        if block == &Block::CALIBRATED_SCULK_SENSOR {
            let mut props = CalibratedSculkSensorLikeProperties::from_state_id(state_id);
            props.sculk_sensor_phase = phase;
            props.power = power;
            props.to_state_id(block)
        } else {
            let mut props = SculkSensorLikeProperties::from_state_id(state_id);
            props.sculk_sensor_phase = phase;
            props.power = power;
            props.to_state_id(block)
        }
    }
}

/// `level.getRandom().nextFloat() * 0.2F + 0.8F`, the clicking sounds' pitch.
fn clicking_pitch() -> f32 {
    rand::rng().random::<f32>() * 0.2 + 0.8
}

impl SculkSensorBlock {
    /// Vanilla `SculkSensorBlock.getPhase`.
    fn get_phase(state: &BlockState) -> SculkSensorPhase {
        SensorProperties::of(Block::from_state_id(state.id), state.id).phase
    }

    /// Vanilla `SculkSensorBlock.canActivate`.
    #[must_use]
    pub fn can_activate(state: &BlockState) -> bool {
        let block = Block::from_state_id(state.id);
        (block == &Block::SCULK_SENSOR || block == &Block::CALIBRATED_SCULK_SENSOR)
            && Self::get_phase(state) == SculkSensorPhase::Inactive
    }

    /// Vanilla `SculkSensorBlock.getActiveTicks`.
    fn get_active_ticks(block: &Block) -> u8 {
        if block == &Block::CALIBRATED_SCULK_SENSOR {
            CALIBRATED_ACTIVE_TICKS
        } else {
            ACTIVE_TICKS
        }
    }

    /// Vanilla `SculkSensorBlock.updateNeighbours`.
    fn update_neighbours(world: &Arc<World>, pos: &BlockPos, block: &Block) {
        world.update_neighbors_at(pos, block, None);
        world.update_neighbors_at(&pos.down(), block, None);
    }

    /// Vanilla `SculkSensorBlock.deactivate`.
    fn deactivate(world: &Arc<World>, pos: &BlockPos, block: &Block, state: &BlockState) {
        world.set_block_state(
            pos,
            SensorProperties::with(block, state.id, SculkSensorPhase::Cooldown, 0),
            BlockFlags::NOTIFY_ALL,
        );
        world.schedule_block_tick(block, *pos, i32::from(COOLDOWN_TICKS), TickPriority::Normal);
        Self::update_neighbours(world, pos, block);
    }

    /// Vanilla `SculkSensorBlock.activate`.
    pub fn activate(
        source_entity: Option<&dyn EntityBase>,
        world: &Arc<World>,
        pos: &BlockPos,
        block: &Block,
        state: &BlockState,
        calculated_power: i32,
        vibration_frequency: i32,
    ) {
        world.set_block_state(
            pos,
            SensorProperties::with(
                block,
                state.id,
                SculkSensorPhase::Active,
                calculated_power as u8,
            ),
            BlockFlags::NOTIFY_ALL,
        );
        world.schedule_block_tick(
            block,
            *pos,
            i32::from(Self::get_active_ticks(block)),
            TickPriority::Normal,
        );
        Self::update_neighbours(world, pos, block);
        Self::try_resonate_vibration(source_entity, world, pos, vibration_frequency);
        world.game_event_from(
            source_entity,
            GameEvent::SculkSensorTendrilsClicking,
            pos.to_centered_f64(),
        );
        if !SensorProperties::of(block, state.id).waterlogged {
            world.play_sound_fine(
                Sound::BlockSculkSensorClicking,
                SoundCategory::Blocks,
                &pos.to_centered_f64(),
                1.0,
                clicking_pitch(),
            );
        }
    }

    /// Vanilla `SculkSensorBlock.tryResonateVibration`: amethyst next to the sensor passes
    /// the frequency on as a `resonate_*` event.
    pub fn try_resonate_vibration(
        source_entity: Option<&dyn EntityBase>,
        world: &World,
        pos: &BlockPos,
        vibration_frequency: i32,
    ) {
        for direction in BlockDirection::all() {
            let relative_pos = pos.offset(direction.to_offset());
            let (block, state) = world.get_block_and_state(&relative_pos);
            if block.has_tag(&tag::Block::MINECRAFT_VIBRATION_RESONATORS) {
                world.game_event(
                    get_resonance_event_by_frequency(vibration_frequency),
                    relative_pos.to_centered_f64(),
                    GameEventContext::of(source_entity, Some(state)),
                );
                world.play_sound_fine(
                    Sound::BlockAmethystBlockResonate,
                    SoundCategory::Blocks,
                    &relative_pos.to_centered_f64(),
                    1.0,
                    get_pitch_from_note(RESONANCE_TONE_MAP[vibration_frequency as usize]),
                );
            }
        }
    }
}

impl BlockBehaviour for SculkSensorBlock {
    /// Vanilla `SculkSensorBlock.getStateForPlacement`, and the calibrated sensor's `FACING`.
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let waterlogged = args.replacing.water_source();
        if args.block == &Block::CALIBRATED_SCULK_SENSOR {
            let mut props = CalibratedSculkSensorLikeProperties::default(args.block);
            props.waterlogged = waterlogged;
            props.facing = args.player.living_entity.entity.get_horizontal_facing();
            props.to_state_id(args.block)
        } else {
            let mut props = SculkSensorLikeProperties::default(args.block);
            props.waterlogged = waterlogged;
            props.to_state_id(args.block)
        }
    }

    /// Vanilla `SculkSensorBlock.onPlace`; Pumpkin only calls it for a new block.
    fn placed(&self, args: PlacedArgs<'_>) {
        let props = SensorProperties::of(args.block, args.state_id);
        if props.power > 0
            && !args
                .world
                .is_block_tick_scheduled(args.position, args.block)
        {
            args.world.set_block_state(
                args.position,
                SensorProperties::with(args.block, args.state_id, props.phase, 0),
                BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
        }
    }

    /// Vanilla `SculkSensorBlock.stepOn`.
    fn on_entity_step(&self, args: OnEntityStepArgs<'_>) {
        if args.below_supporting_block
            || !Self::can_activate(args.state)
            || args.entity.get_entity().entity_type == &EntityType::WARDEN
        {
            return;
        }
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return;
        };
        let Some(sensor) = SculkSensorBlockEntity::of(block_entity.as_ref()) else {
            return;
        };
        if vibrations::VibrationUser::can_receive_vibration(
            sensor,
            args.world,
            *args.position,
            GameEvent::Step,
            &GameEventContext::of_state(BlockState::from_id(args.state.id)),
        ) {
            vibrations::force_schedule_vibration(
                sensor,
                args.world,
                GameEvent::Step,
                &GameEventContext::of_entity(args.entity),
                args.entity.get_entity().pos.load(),
            );
        }
    }

    /// Vanilla `SculkSensorBlock.tick`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state = args.world.get_block_state(args.position);
        let props = SensorProperties::of(args.block, state.id);
        match props.phase {
            SculkSensorPhase::Active => {
                Self::deactivate(args.world, args.position, args.block, state);
            }
            SculkSensorPhase::Cooldown => {
                args.world.set_block_state(
                    args.position,
                    SensorProperties::with(
                        args.block,
                        state.id,
                        SculkSensorPhase::Inactive,
                        props.power,
                    ),
                    BlockFlags::NOTIFY_ALL,
                );
                if !props.waterlogged {
                    args.world.play_sound_fine(
                        Sound::BlockSculkSensorClickingStop,
                        SoundCategory::Blocks,
                        &args.position.to_centered_f64(),
                        1.0,
                        clicking_pitch(),
                    );
                }
            }
            SculkSensorPhase::Inactive => {}
        }
    }

    /// Vanilla `SculkSensorBlock.affectNeighborsAfterRemoval`.
    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        if SensorProperties::of(args.block, args.old_state_id).phase == SculkSensorPhase::Active {
            Self::update_neighbours(args.world, args.position, args.block);
        }
    }

    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    /// Vanilla `SculkSensorBlock.ownSignal`, which the calibrated sensor withholds from the
    /// block it faces (`CalibratedSculkSensorBlock.getSignal`).
    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        if calibrated_facing(args.state) == Some(args.direction) {
            return 0;
        }
        SensorProperties::of(args.block, args.state.id).power
    }

    /// Vanilla `SculkSensorBlock.getDirectSignal`.
    fn get_strong_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        if args.direction == BlockDirection::Up {
            self.get_weak_redstone_power(args)
        } else {
            0
        }
    }

    /// Vanilla `SculkSensorBlock.getAnalogOutputSignal`.
    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        let frequency = args
            .world
            .get_block_entity(args.position)
            .and_then(|block_entity| {
                SculkSensorBlockEntity::of(block_entity.as_ref())
                    .map(SculkSensorBlockEntity::get_last_vibration_frequency)
            })
            .filter(|_| Self::get_phase(args.state) == SculkSensorPhase::Active);
        Some(frequency.map_or(0, |frequency| frequency as u8))
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
