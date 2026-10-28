//! The parts of vanilla `BaseRailBlock` every rail shares.

use std::sync::Arc;

use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    block_properties::{HorizontalFacing, RailShape},
    fluid::Fluid,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

use crate::block::{OnPlaceArgs, drop_loot};
use crate::entity::EntityBase;
use crate::world::World;
use crate::world::loot::LootContextParameters;

use super::super::block_receives_redstone_power;
use super::{RailProperties, RailState};

/// Vanilla `BaseRailBlock.canSurvive`.
pub(super) fn can_place_rail_at(world: &dyn BlockAccessor, pos: &BlockPos) -> bool {
    let state = world.get_block_state(&pos.down());
    state.is_side_solid(BlockDirection::Up)
}

/// Vanilla `BaseRailBlock.shouldBeRemoved`.
fn should_be_removed(world: &World, pos: &BlockPos, shape: RailShape) -> bool {
    if !can_place_rail_at(world, pos) {
        return true;
    }

    let slope_support = match shape {
        RailShape::AscendingEast => pos.east(),
        RailShape::AscendingWest => pos.west(),
        RailShape::AscendingNorth => pos.north(),
        RailShape::AscendingSouth => pos.south(),
        _ => return false,
    };
    !can_place_rail_at(world, &slope_support.up())
}

/// Vanilla `BaseRailBlock.getStateForPlacement`: the shape follows the player's horizontal
/// direction, and the rail is waterlogged in a water source.
pub(super) fn rail_state_for_placement(args: &OnPlaceArgs<'_>) -> BlockStateId {
    let mut rail_props = RailProperties::default(args.block);
    let shape = match args.player.get_entity().get_horizontal_facing() {
        HorizontalFacing::East | HorizontalFacing::West => RailShape::EastWest,
        HorizontalFacing::North | HorizontalFacing::South => RailShape::NorthSouth,
    };
    rail_props.set_shape(shape);
    rail_props.set_waterlogged(args.replacing.water_source());
    rail_props.to_state_id(args.block)
}

/// Vanilla `BaseRailBlock.updateDir`.
pub(super) fn update_dir(
    world: &Arc<World>,
    pos: &BlockPos,
    state_id: BlockStateId,
    first: bool,
) -> BlockStateId {
    let rail = RailState::new(*pos, state_id);
    let current = rail.properties.shape();
    rail.place(
        world,
        block_receives_redstone_power(world, pos),
        first,
        current,
    )
    .state_id()
}

/// Vanilla `BaseRailBlock.onPlace` and its `updateState(state, level, pos, movedByPiston)`:
/// a new rail picks its shape, and a straight rail then checks its power as a neighbour
/// update. Returns the updated state.
pub(super) fn on_place(
    world: &Arc<World>,
    pos: &BlockPos,
    block: &Block,
    state_id: BlockStateId,
) -> BlockStateId {
    let state_id = update_dir(world, pos, state_id, true);
    if !RailProperties::new(state_id, block).can_curve() {
        world.update_neighbor(pos, block);
    }
    state_id
}

/// The first half of vanilla `BaseRailBlock.neighborChanged`: a rail that lost its support
/// drops and is removed. Returns false when the rail is gone.
pub(super) fn survives_neighbor_change(world: &Arc<World>, pos: &BlockPos, block: &Block) -> bool {
    let (current_block, state) = world.get_block_and_state(pos);
    if current_block != block {
        return false;
    }
    let shape = RailProperties::new(state.id, block).shape();
    if !should_be_removed(world, pos, shape) {
        return true;
    }

    // Vanilla `dropResources` then `removeBlock(pos, false)`: no break event or particles.
    let params = LootContextParameters {
        block_state: Some(state),
        position: Some(pos.to_f64()),
        ..Default::default()
    };
    drop_loot(world, block, pos, true, &params);
    let left_behind = if block.is_waterlogged(state.id) {
        Block::WATER.default_state.id
    } else {
        BlockStateId::AIR
    };
    world.set_block_state(pos, left_behind, BlockFlags::NOTIFY_ALL);
    false
}

/// Vanilla `BaseRailBlock.updateShape`: a waterlogged rail schedules a water tick.
pub(super) fn schedule_water_tick(
    world: &World,
    pos: &BlockPos,
    block: &Block,
    state_id: BlockStateId,
) {
    if RailProperties::new(state_id, block).is_waterlogged() {
        world.schedule_fluid_tick(
            &Fluid::WATER,
            *pos,
            Fluid::WATER.flow_speed as i32,
            TickPriority::Normal,
        );
    }
}

/// Vanilla `BaseRailBlock.affectNeighborsAfterRemoval`.
pub(super) fn affect_neighbors_after_removal(
    world: &Arc<World>,
    pos: &BlockPos,
    block: &Block,
    old_state_id: BlockStateId,
    moved: bool,
) {
    if moved {
        return;
    }
    let rail_props = RailProperties::new(old_state_id, block);
    if rail_props.shape().is_ascending() {
        world.update_neighbors_at(&pos.up(), block, None);
    }
    if !rail_props.can_curve() {
        world.update_neighbors_at(pos, block, None);
        world.update_neighbors_at(&pos.down(), block, None);
    }
}
