use std::sync::Arc;

use pumpkin_data::game_event::GameEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::BlockEntity;
use super::sculk_sensor::SculkSensorBlockEntity;
use crate::world::World;
use crate::world::game_event::{GameEventContext, GameEventListener, PositionSource};

/// Vanilla `CalibratedSculkSensorBlockEntity`: a sculk sensor whose vibration user has the
/// calibrated overrides.
pub struct CalibratedSculkSensorBlockEntity {
    pub sensor: SculkSensorBlockEntity,
}

impl BlockEntity for CalibratedSculkSensorBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.sensor.position
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let sensor = Self::new(position).sensor;
        sensor.load_additional(nbt);
        Self { sensor }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.sensor.write_nbt(nbt);
    }

    fn tick(&self, world: &Arc<World>) {
        self.sensor.tick(world);
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        self.sensor.chunk_data_nbt()
    }

    fn get_listener(self: Arc<Self>) -> Option<Arc<dyn GameEventListener>> {
        Some(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl GameEventListener for CalibratedSculkSensorBlockEntity {
    fn get_listener_source(&self) -> &PositionSource {
        self.sensor.get_listener_source()
    }

    fn get_listener_radius(&self) -> i32 {
        GameEventListener::get_listener_radius(&self.sensor)
    }

    fn handle_game_event(
        &self,
        world: &World,
        event: GameEvent,
        context: &GameEventContext<'_>,
        source_position: Vector3<f64>,
    ) -> bool {
        self.sensor
            .handle_game_event(world, event, context, source_position)
    }
}

impl CalibratedSculkSensorBlockEntity {
    pub const ID: &'static str = "minecraft:calibrated_sculk_sensor";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            sensor: SculkSensorBlockEntity::with_calibration(position, true),
        }
    }
}
