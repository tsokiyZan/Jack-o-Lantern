use pumpkin_data::block_properties::PaleHangingMossLikeProperties;
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs, OnScheduledTickArgs,
};

/// Vanilla `HangingMossBlock`.
#[pumpkin_block("minecraft:pale_hanging_moss")]
pub struct HangingMossBlock;

impl HangingMossBlock {
    /// Vanilla `HangingMossBlock.canStayAtPosition`.
    fn can_stay_at_position(world: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        let (above_block, above_state) = world.get_block_and_state(&pos.up());
        can_attach_below(above_state) || above_block == &Block::PALE_HANGING_MOSS
    }

    fn with_tip(block: &Block, state_id: BlockStateId, below: &Block) -> BlockStateId {
        let mut props = PaleHangingMossLikeProperties::from_state_id(state_id);
        props.tip = below != &Block::PALE_HANGING_MOSS;
        props.to_state_id(block)
    }
}

/// Vanilla `MultifaceBlock.canAttachTo` for something hanging under `state`: its
/// bottom face must be full in its support shape or its collision shape. The
/// collision shape is what lets moss hang from leaves.
fn can_attach_below(state: &BlockState) -> bool {
    state.is_side_solid(BlockDirection::Down)
        || state.get_block_collision_shapes().any(|shape| {
            shape.min.y <= 0.0
                && shape.min.x <= 0.0
                && shape.min.z <= 0.0
                && shape.max.x >= 1.0
                && shape.max.z >= 1.0
        })
}

impl BlockBehaviour for HangingMossBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        Self::can_stay_at_position(args.block_accessor, args.position)
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let below = args.world.get_block(&args.position.down());
        Self::with_tip(args.block, args.block.default_state.id, below)
    }

    /// Vanilla `HangingMossBlock.updateShape`: losing support schedules a break
    /// next tick instead of breaking at once.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if !Self::can_stay_at_position(args.world, args.position) {
            args.world
                .schedule_block_tick(args.block, *args.position, 1, TickPriority::Normal);
        }
        let below = args.world.get_block(&args.position.down());
        Self::with_tip(args.block, args.state_id, below)
    }

    /// Vanilla `HangingMossBlock.tick`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        if !Self::can_stay_at_position(args.world.as_ref(), args.position) {
            args.world
                .break_block(args.position, None, BlockFlags::NOTIFY_ALL);
        }
    }
}
