use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use pumpkin_data::game_event::GameEvent;
use pumpkin_data::{Block, BlockDirection};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::BlockEntity;
use super::calibrated_sculk_sensor::CalibratedSculkSensorBlockEntity;
use crate::block::blocks::redstone::get_redstone_power;
use crate::block::blocks::redstone::sculk_sensor::{SculkSensorBlock, calibrated_facing};
use crate::entity::EntityBase;
use crate::world::World;
use crate::world::game_event::vibrations::{
    self, VibrationData, VibrationSystem, VibrationUser, get_game_event_frequency,
    get_redstone_strength_for_distance,
};
use crate::world::game_event::{GameEventContext, GameEventListener, PositionSource};

/// Vanilla `SculkSensorBlockEntity.VibrationUser.LISTENER_RANGE`.
pub const LISTENER_RANGE: i32 = 8;
/// The radius vanilla `CalibratedSculkSensorBlockEntity.VibrationUser` listens in.
pub const CALIBRATED_LISTENER_RANGE: i32 = 16;

/// Vanilla `SculkSensorBlockEntity`. `calibrated` gives its user the overrides of vanilla
/// `CalibratedSculkSensorBlockEntity.VibrationUser`.
pub struct SculkSensorBlockEntity {
    pub position: BlockPos,
    calibrated: bool,
    /// Vanilla `SculkSensorBlockEntity.VibrationUser.positionSource`.
    position_source: PositionSource,
    last_vibration_frequency: AtomicI32,
    vibration_data: Mutex<VibrationData>,
}

impl BlockEntity for SculkSensorBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let sensor = Self::new(position);
        sensor.load_additional(nbt);
        sensor
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int(
            "last_vibration_frequency",
            self.get_last_vibration_frequency(),
        );
        nbt.put_compound(
            VibrationData::NBT_TAG_KEY,
            self.vibration_data
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .write_nbt(),
        );
    }

    /// Vanilla `SculkSensorBlock.getTicker`.
    fn tick(&self, world: &Arc<World>) {
        vibrations::tick(world, &self.vibration_data, self);
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        nbt.put_int(
            "last_vibration_frequency",
            self.get_last_vibration_frequency(),
        );
        Some(nbt)
    }

    fn get_listener(self: Arc<Self>) -> Option<Arc<dyn GameEventListener>> {
        Some(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl SculkSensorBlockEntity {
    pub const ID: &'static str = "minecraft:sculk_sensor";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self::with_calibration(position, false)
    }

    pub(super) fn with_calibration(position: BlockPos, calibrated: bool) -> Self {
        Self {
            position,
            calibrated,
            position_source: PositionSource::Block(position),
            last_vibration_frequency: AtomicI32::new(0),
            vibration_data: Mutex::new(VibrationData::default()),
        }
    }

    /// Vanilla `SculkSensorBlockEntity.loadAdditional`.
    pub(super) fn load_additional(&self, nbt: &NbtCompound) {
        self.last_vibration_frequency.store(
            nbt.get_int("last_vibration_frequency").unwrap_or(0),
            Ordering::Relaxed,
        );
        *self
            .vibration_data
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = VibrationData::read_nbt(nbt);
    }

    /// The sensor behind a sculk sensor or calibrated sculk sensor block entity.
    #[must_use]
    pub fn of(block_entity: &dyn BlockEntity) -> Option<&Self> {
        let any = block_entity.as_any();
        any.downcast_ref::<Self>().or_else(|| {
            any.downcast_ref::<CalibratedSculkSensorBlockEntity>()
                .map(|calibrated| &calibrated.sensor)
        })
    }

    #[must_use]
    pub fn get_last_vibration_frequency(&self) -> i32 {
        self.last_vibration_frequency.load(Ordering::Relaxed)
    }

    /// Vanilla `CalibratedSculkSensorBlockEntity.VibrationUser.getBackSignal`.
    fn get_back_signal(&self, world: &World, facing: BlockDirection) -> i32 {
        let direction = facing.opposite();
        let back_pos = self.position.offset(direction.to_offset());
        let (block, state) = world.get_block_and_state(&back_pos);
        i32::from(get_redstone_power(
            block, state, world, &back_pos, direction,
        ))
    }
}

impl GameEventListener for SculkSensorBlockEntity {
    fn get_listener_source(&self) -> &PositionSource {
        &self.position_source
    }

    fn get_listener_radius(&self) -> i32 {
        VibrationUser::get_listener_radius(self)
    }

    fn handle_game_event(
        &self,
        world: &World,
        event: GameEvent,
        context: &GameEventContext<'_>,
        source_position: Vector3<f64>,
    ) -> bool {
        vibrations::handle_game_event(self, world, event, context, source_position)
    }
}

impl VibrationSystem for SculkSensorBlockEntity {
    fn get_vibration_data(&self) -> &Mutex<VibrationData> {
        &self.vibration_data
    }

    fn get_vibration_user(&self) -> &dyn VibrationUser {
        self
    }
}

/// Vanilla `SculkSensorBlockEntity.VibrationUser`, with
/// `CalibratedSculkSensorBlockEntity.VibrationUser`'s overrides when calibrated.
impl VibrationUser for SculkSensorBlockEntity {
    fn get_listener_radius(&self) -> i32 {
        if self.calibrated {
            CALIBRATED_LISTENER_RANGE
        } else {
            LISTENER_RANGE
        }
    }

    fn get_position_source(&self) -> &PositionSource {
        &self.position_source
    }

    fn can_trigger_avoid_vibration(&self) -> bool {
        true
    }

    fn can_receive_vibration(
        &self,
        world: &World,
        pos: BlockPos,
        event: GameEvent,
        _context: &GameEventContext<'_>,
    ) -> bool {
        let state = world.get_block_state(&self.position);
        if self.calibrated
            && let Some(facing) = calibrated_facing(state)
        {
            let comparison_type = self.get_back_signal(world, facing);
            if comparison_type != 0 && get_game_event_frequency(event) != comparison_type {
                return false;
            }
        }
        if pos == self.position && matches!(event, GameEvent::BlockDestroy | GameEvent::BlockPlace)
        {
            return false;
        }
        get_game_event_frequency(event) != 0 && SculkSensorBlock::can_activate(state)
    }

    fn on_receive_vibration(
        &self,
        world: &Arc<World>,
        _pos: BlockPos,
        event: GameEvent,
        source_entity: Option<&dyn EntityBase>,
        _projectile_owner: Option<&dyn EntityBase>,
        receiving_distance: f32,
    ) {
        let (block, state) = world.get_block_and_state(&self.position);
        if !SculkSensorBlock::can_activate(state) {
            return;
        }
        let event_frequency = get_game_event_frequency(event);
        self.last_vibration_frequency
            .store(event_frequency, Ordering::Relaxed);
        let calculated_power = get_redstone_strength_for_distance(
            receiving_distance,
            VibrationUser::get_listener_radius(self),
        );
        if block == &Block::SCULK_SENSOR || block == &Block::CALIBRATED_SCULK_SENSOR {
            SculkSensorBlock::activate(
                source_entity,
                world,
                &self.position,
                block,
                state,
                calculated_power,
                event_frequency,
            );
        }
    }

    fn requires_adjacent_chunks_to_be_ticking(&self) -> bool {
        true
    }
}
