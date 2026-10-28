use pumpkin_data::BlockStateId;
use pumpkin_macros::pumpkin_block;

use super::common::{
    affect_neighbors_after_removal, can_place_rail_at, on_place, rail_state_for_placement,
    schedule_water_tick, survives_neighbor_change,
};
use super::powered_rail::update_state;
use crate::block::BlockBehaviour;
use crate::block::CanPlaceAtArgs;
use crate::block::GetStateForNeighborUpdateArgs;
use crate::block::OnNeighborUpdateArgs;
use crate::block::OnPlaceArgs;
use crate::block::OnStateReplacedArgs;
use crate::block::PlacedArgs;

/// Vanilla `PoweredRailBlock`, which the activator rail also is.
#[pumpkin_block("minecraft:activator_rail")]
pub struct ActivatorRailBlock;

impl BlockBehaviour for ActivatorRailBlock {
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
