use std::sync::Arc;

use pumpkin_data::block_properties::SuspiciousSandLikeProperties;
use pumpkin_data::{BlockId, BlockStateId};

use crate::block::blocks::falling::FallingBlock;
use crate::block::entities::brushable_block::BrushableBlockBlockEntity;
use crate::block::{
    BlockBehaviour, BlockMetadata, BrokenArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs,
    OnScheduledTickArgs, PlacedArgs,
};
use crate::entity::falling::FallingEntity;
use pumpkin_world::tick::TickPriority;

pub struct BrushableBlock;

impl BlockMetadata for BrushableBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::SUSPICIOUS_SAND, BlockId::SUSPICIOUS_GRAVEL].into()
    }
}

impl BlockBehaviour for BrushableBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let props = SuspiciousSandLikeProperties::default(args.block);
        props.to_state_id(args.block)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if args.is_new_block() {
            let entity = BrushableBlockBlockEntity::new(*args.position);
            args.world.add_block_entity(Arc::new(entity));
        }
        // Vanilla `BrushableBlock.onPlace`: every state change (each dusted stage) reschedules.
        args.world
            .schedule_block_tick(args.block, *args.position, 2, TickPriority::Normal);
    }

    /// Vanilla `BrushableBlock.updateShape`: a neighbour change schedules the tick.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        args.world
            .schedule_block_tick(args.block, *args.position, 2, TickPriority::Normal);
        args.state_id
    }

    /// Vanilla `BrushableBlock.tick`: the block entity's `checkReset`, then a fall, which
    /// never drops the block, when the block below is free.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        if let Some(be) = args.world.get_block_entity(args.position)
            && let Some(brushable) = be.as_any().downcast_ref::<BrushableBlockBlockEntity>()
        {
            brushable.check_reset(args.world);
        }
        let (below_block, below_state) = args.world.get_block_and_state(&args.position.down());
        if FallingBlock::can_fall_through(below_state, below_block)
            && args.position.0.y >= args.world.min_y
        {
            let state = args.world.get_block_state(args.position);
            FallingEntity::replace_spawn(args.world, *args.position, state.id).disable_drop();
        }
    }

    fn broken(&self, args: BrokenArgs<'_>) {
        if let Some(be) = args.world.get_block_entity(args.position)
            && let Some(brush_be) = be.as_any().downcast_ref::<BrushableBlockBlockEntity>()
            && let Some(contained) = brush_be
                .item
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
        {
            args.world.drop_stack(args.position, contained);
        }
    }
}
