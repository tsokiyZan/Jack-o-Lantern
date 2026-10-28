use pumpkin_data::{BlockDirection, BlockStateId};
use pumpkin_macros::pumpkin_block;

use crate::block::BlockBehaviour;
use crate::block::CanPlaceAtArgs;
use crate::block::GetStateForNeighborUpdateArgs;
use crate::block::OnNeighborUpdateArgs;
use crate::block::OnPlaceArgs;
use crate::block::OnStateReplacedArgs;
use crate::block::PlacedArgs;

use super::RailState;
use super::common::{
    affect_neighbors_after_removal, can_place_rail_at, on_place, rail_state_for_placement,
    schedule_water_tick, survives_neighbor_change, update_dir,
};

#[pumpkin_block("minecraft:rail")]
pub struct RailBlock;

impl BlockBehaviour for RailBlock {
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

    /// Vanilla `RailBlock.updateState`: a rail with three potential connections re-picks its
    /// shape when a signal source changes next to it.
    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        if !survives_neighbor_change(args.world, args.position, args.block) {
            return;
        }
        // Vanilla `defaultBlockState().isSignalSource()`. The fork's check takes a side, and
        // diodes answer only for their axis, so any side counts.
        let source = args.source_block;
        let is_signal_source = BlockDirection::all().into_iter().any(|direction| {
            args.world
                .block_registry
                .emits_redstone_power(source, source.default_state, direction)
        });
        if !is_signal_source {
            return;
        }
        let state_id = args.world.get_block_state_id(args.position);
        let rail = RailState::new(*args.position, state_id);
        if rail.count_potential_connections(args.world) == 3 {
            update_dir(args.world, args.position, state_id, false);
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
