use std::sync::Arc;

use crate::block::blocks::weathering_copper::{
    ChangeOverTimeBlock, WeatherState, WeatheringCopper, change_over_time, get_chance_modifier,
    get_first, get_next, get_previous, get_weather_state,
};
use crate::block::{
    BlockBehaviour, EmitsRedstonePowerArgs, GetRedstonePowerArgs, GetStateForNeighborUpdateArgs,
    OnPlaceArgs, OnScheduledTickArgs, OnStateReplacedArgs, PathComputationType, PlacedArgs,
    RandomTickArgs,
};
use crate::world::World;
use pumpkin_data::block_properties::{Axis, LightningRodLikeProperties};
use pumpkin_data::fluid::Fluid;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockState, BlockStateId, FacingExt};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

/// Vanilla `LightningRodBlock.ACTIVATION_TICKS`.
const ACTIVATION_TICKS: i32 = 8;

/// Vanilla `LightningRodBlock` and `WeatheringLightningRodBlock`: every copper lightning rod,
/// waxed or not.
#[pumpkin_block_from_tag("minecraft:lightning_rods")]
pub struct LightningRodBlock;

impl ChangeOverTimeBlock<WeatherState> for LightningRodBlock {
    fn get_age(&self, block: &Block) -> Option<WeatherState> {
        get_weather_state(block)
    }

    fn get_chance_modifier(&self, age: WeatherState) -> f32 {
        get_chance_modifier(age)
    }

    fn get_next(&self, block: &Block) -> Option<&'static Block> {
        get_next(block)
    }

    fn get_previous(&self, block: &Block) -> Option<&'static Block> {
        get_previous(block)
    }

    fn get_first(&self, block: &Block) -> Option<&'static Block> {
        get_first(block)
    }
}

impl WeatheringCopper for LightningRodBlock {}

impl LightningRodBlock {
    /// Vanilla `LightningRodBlock.onLightningStrike`.
    pub fn trigger(world: &Arc<World>, pos: &BlockPos) {
        let (block, state_id) = world.get_block_and_state_id(pos);
        let mut props = LightningRodLikeProperties::from_state_id(state_id);
        props.powered = true;
        world.set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL);
        Self::update_neighbors(world, pos, block, props);
        world.schedule_block_tick(block, *pos, ACTIVATION_TICKS, TickPriority::Normal);
        let axis = match props.facing.to_block_direction().to_axis() {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        };
        world.sync_world_event(WorldEvent::ParticlesElectricSpark, *pos, axis);
    }

    /// Vanilla `LightningRodBlock.updateNeighbours`: only the block the rod is attached to.
    fn update_neighbors(
        world: &Arc<World>,
        pos: &BlockPos,
        block: &Block,
        props: LightningRodLikeProperties,
    ) {
        let attached_pos = pos.offset(props.facing.opposite().to_block_direction().to_offset());
        world.update_neighbors_at(&attached_pos, block, None);
    }
}

impl BlockBehaviour for LightningRodBlock {
    /// Vanilla `LightningRodBlock.onPlace`: a powered rod without a pending tick gets one.
    fn placed(&self, args: PlacedArgs<'_>) {
        if args.is_new_block()
            && LightningRodLikeProperties::from_state_id(args.state_id).powered
            && !args
                .world
                .is_block_tick_scheduled(args.position, args.block)
        {
            args.world.schedule_block_tick(
                args.block,
                *args.position,
                ACTIVATION_TICKS,
                TickPriority::Normal,
            );
        }
    }

    /// Vanilla `LightningRodBlock.getStateForPlacement`.
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = LightningRodLikeProperties::default(args.block);
        props.facing = args.direction.to_facing().opposite();
        props.waterlogged = args.replacing.water_source();
        props.to_state_id(args.block)
    }

    /// Vanilla `LightningRodBlock.updateShape`: a waterlogged rod schedules a water tick.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if LightningRodLikeProperties::from_state_id(args.state_id).waterlogged {
            args.world.schedule_fluid_tick(
                &Fluid::WATER,
                *args.position,
                Fluid::WATER.flow_speed as i32,
                TickPriority::Normal,
            );
        }
        args.state_id
    }

    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let props = LightningRodLikeProperties::from_state_id(args.state.id);
        if props.powered { 15 } else { 0 }
    }

    fn get_strong_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let props = LightningRodLikeProperties::from_state_id(args.state.id);
        // It emits strong power only in its facing direction (the direction pointing outward)
        if props.powered && props.facing.to_block_direction() == args.direction {
            15
        } else {
            0
        }
    }

    /// Vanilla `LightningRodBlock.tick`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state = args.world.get_block_state(args.position);
        let mut props = LightningRodLikeProperties::from_state_id(state.id);
        props.powered = false;
        args.world.set_block_state(
            args.position,
            props.to_state_id(args.block),
            BlockFlags::NOTIFY_ALL,
        );
        Self::update_neighbors(args.world, args.position, args.block, props);
    }

    /// Vanilla `WeatheringLightningRodBlock.randomTick`.
    fn random_tick(&self, args: RandomTickArgs<'_>) {
        change_over_time(args.world, args.position, args.block);
    }

    /// Vanilla `LightningRodBlock.affectNeighborsAfterRemoval`, moved by a piston or not.
    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        let props = LightningRodLikeProperties::from_state_id(args.old_state_id);
        if props.powered {
            Self::update_neighbors(args.world, args.position, args.block, props);
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
