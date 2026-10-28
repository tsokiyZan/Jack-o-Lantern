use pumpkin_data::BlockStateId;
use pumpkin_data::block_properties::RailShape;
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

use crate::block::BlockBehaviour;
use crate::block::CanPlaceAtArgs;
use crate::block::GetStateForNeighborUpdateArgs;
use crate::block::OnNeighborUpdateArgs;
use crate::block::OnPlaceArgs;
use crate::block::OnStateReplacedArgs;
use crate::block::PlacedArgs;
use crate::world::World;
use pumpkin_data::Block;
use pumpkin_util::math::position::BlockPos;

use super::super::block_receives_redstone_power;
use super::RailProperties;
use super::common::{
    affect_neighbors_after_removal, can_place_rail_at, on_place, rail_state_for_placement,
    schedule_water_tick, survives_neighbor_change,
};

#[pumpkin_block("minecraft:powered_rail")]
pub struct PoweredRailBlock;

impl BlockBehaviour for PoweredRailBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        rail_state_for_placement(&args)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        on_place(args.world, args.position, args.block, args.state_id);
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        schedule_water_tick(args.world, args.position, args.block, args.state_id);
        args.state_id
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        if survives_neighbor_change(args.world, args.position, args.block) {
            update_state(args.world, args.block, args.position);
        }
    }

    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        affect_neighbors_after_removal(
            args.world,
            args.position,
            args.block,
            args.old_state_id,
            args.moved,
        );
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_place_rail_at(args.block_accessor, args.position)
    }
}

/// Vanilla `PoweredRailBlock.findPoweredRailSignal`, shared with the activator rail.
fn find_powered_rail_signal(
    world: &World,
    block: &Block,
    pos: &BlockPos,
    shape: RailShape,
    forward: bool,
    search_depth: u8,
) -> bool {
    if search_depth >= 8 {
        return false;
    }

    let mut x = pos.0.x;
    let mut y = pos.0.y;
    let mut z = pos.0.z;
    let mut check_below = true;
    let mut shape = shape;
    match shape {
        RailShape::NorthSouth => {
            if forward {
                z += 1;
            } else {
                z -= 1;
            }
        }
        RailShape::EastWest => {
            if forward {
                x -= 1;
            } else {
                x += 1;
            }
        }
        RailShape::AscendingEast => {
            if forward {
                x -= 1;
            } else {
                x += 1;
                y += 1;
                check_below = false;
            }
            shape = RailShape::EastWest;
        }
        RailShape::AscendingWest => {
            if forward {
                x -= 1;
                y += 1;
                check_below = false;
            } else {
                x += 1;
            }
            shape = RailShape::EastWest;
        }
        RailShape::AscendingNorth => {
            if forward {
                z += 1;
            } else {
                z -= 1;
                y += 1;
                check_below = false;
            }
            shape = RailShape::NorthSouth;
        }
        RailShape::AscendingSouth => {
            if forward {
                z += 1;
                y += 1;
                check_below = false;
            } else {
                z -= 1;
            }
            shape = RailShape::NorthSouth;
        }
        // Straight rails have no curved shapes.
        _ => {}
    }

    is_same_rail_with_power(
        world,
        block,
        &BlockPos::new(x, y, z),
        forward,
        search_depth,
        shape,
    ) || check_below
        && is_same_rail_with_power(
            world,
            block,
            &BlockPos::new(x, y - 1, z),
            forward,
            search_depth,
            shape,
        )
}

/// Vanilla `PoweredRailBlock.isSameRailWithPower`.
fn is_same_rail_with_power(
    world: &World,
    block: &Block,
    pos: &BlockPos,
    forward: bool,
    search_depth: u8,
    dir: RailShape,
) -> bool {
    let (other_block, state_id) = world.get_block_and_state_id(pos);
    if other_block != block {
        return false;
    }

    let rail_props = RailProperties::new(state_id, block);
    let my_shape = rail_props.shape();
    if dir == RailShape::EastWest
        && matches!(
            my_shape,
            RailShape::NorthSouth | RailShape::AscendingNorth | RailShape::AscendingSouth
        )
    {
        return false;
    }
    if dir == RailShape::NorthSouth
        && matches!(
            my_shape,
            RailShape::EastWest | RailShape::AscendingEast | RailShape::AscendingWest
        )
    {
        return false;
    }
    if !rail_props.is_powered() {
        return false;
    }

    block_receives_redstone_power(world, pos)
        || find_powered_rail_signal(world, block, pos, my_shape, forward, search_depth + 1)
}

/// Vanilla `PoweredRailBlock.updateState`, shared with the activator rail: power comes from
/// a neighbour signal or a powered rail of the same kind up to eight rails away.
pub(super) fn update_state(world: &Arc<World>, block: &Block, pos: &BlockPos) {
    let state_id = world.get_block_state_id(pos);
    let mut rail_props = RailProperties::new(state_id, block);
    let shape = rail_props.shape();
    let is_powered = rail_props.is_powered();
    let should_power = block_receives_redstone_power(world, pos)
        || find_powered_rail_signal(world, block, pos, shape, true, 0)
        || find_powered_rail_signal(world, block, pos, shape, false, 0);

    if should_power != is_powered {
        rail_props.set_powered(should_power);
        world.set_block_state(pos, rail_props.to_state_id(block), BlockFlags::NOTIFY_ALL);
        world.update_neighbors_at(&pos.down(), block, None);
        if shape.is_ascending() {
            world.update_neighbors_at(&pos.up(), block, None);
        }
    }
}
