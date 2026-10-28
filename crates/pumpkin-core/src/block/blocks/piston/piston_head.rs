use pumpkin_data::block_properties::{MovingPistonLikeProperties, PistonType};
use pumpkin_data::{Block, BlockState, BlockStateId, FacingExt};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnNeighborUpdateArgs,
    OnStateReplacedArgs, PathComputationType, PlayerWillDestroyArgs,
};

use super::piston::PistonProps;

pub(crate) type PistonHeadProperties = pumpkin_data::block_properties::PistonHeadLikeProperties;

#[pumpkin_block("minecraft:piston_head")]
pub struct PistonHeadBlock;

/// Vanilla `PistonHeadBlock.isFittingBase`: an extended piston of the arm's type facing the
/// same way.
fn is_fitting_base(
    head: PistonHeadProperties,
    base_block: &Block,
    base_state: BlockStateId,
) -> bool {
    let expected_base = if head.r#type == PistonType::Normal {
        &Block::PISTON
    } else {
        &Block::STICKY_PISTON
    };
    if base_block != expected_base {
        return false;
    }
    let base = PistonProps::from_state_id(base_state);
    base.extended && base.facing == head.facing
}

/// The block the arm sits on: one step against its facing.
fn base_pos(head: PistonHeadProperties, pos: &BlockPos) -> BlockPos {
    pos.offset(head.facing.opposite().to_block_direction().to_offset())
}

/// Vanilla `PistonHeadBlock.canSurvive`: a fitting base, or a moving piston facing the same
/// way.
fn can_survive(accessor: &dyn BlockAccessor, pos: &BlockPos, state_id: BlockStateId) -> bool {
    let head = PistonHeadProperties::from_state_id(state_id);
    let (base_block, base_state) = accessor.get_block_and_state(&base_pos(head, pos));
    is_fitting_base(head, base_block, base_state.id)
        || base_block == &Block::MOVING_PISTON
            && MovingPistonLikeProperties::from_state_id(base_state.id).facing == head.facing
}

impl BlockBehaviour for PistonHeadBlock {
    /// Vanilla `PistonHeadBlock.playerWillDestroy`: a player who gets no drops takes the base
    /// with the arm, without its drops.
    fn player_will_destroy(&self, args: PlayerWillDestroyArgs<'_>) {
        // Vanilla `Player.preventsBlockDrops`: the `instabuild` ability.
        if !args
            .player
            .abilities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .creative
        {
            return;
        }
        let head = PistonHeadProperties::from_state_id(args.state.id);
        let base_pos = base_pos(head, args.position);
        let (base_block, base_state) = args.world.get_block_and_state_id(&base_pos);
        if is_fitting_base(head, base_block, base_state) {
            args.world.break_block(
                &base_pos,
                None,
                BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
            );
        }
    }

    /// Vanilla `PistonHeadBlock.affectNeighborsAfterRemoval`: the base goes with the arm.
    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        let head = PistonHeadProperties::from_state_id(args.old_state_id);
        let base_pos = base_pos(head, args.position);
        let (base_block, base_state) = args.world.get_block_and_state_id(&base_pos);
        if is_fitting_base(head, base_block, base_state) {
            args.world
                .break_block(&base_pos, None, BlockFlags::NOTIFY_ALL);
        }
    }

    /// Vanilla `PistonHeadBlock.updateShape`: the arm goes when its base does.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let head = PistonHeadProperties::from_state_id(args.state_id);
        if args.direction.opposite() == head.facing.to_block_direction()
            && !can_survive(args.world, args.position, args.state_id)
        {
            Block::AIR.default_state.id
        } else {
            args.state_id
        }
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_survive(args.block_accessor, args.position, args.state.id)
    }

    /// Vanilla `PistonHeadBlock.neighborChanged`: a surviving arm passes the update on to its
    /// base.
    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        if can_survive(args.world.as_ref(), args.position, state_id) {
            let head = PistonHeadProperties::from_state_id(state_id);
            args.world
                .update_neighbor(&base_pos(head, args.position), args.source_block);
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
