use std::sync::Arc;

use pumpkin_data::block_properties::SuspiciousSandLikeProperties;
use pumpkin_data::{BlockId, BlockStateId};

use crate::block::entities::brushable_block::BrushableBlockBlockEntity;
use crate::block::{
    BlockBehaviour, BlockMetadata, BrokenArgs, OnPlaceArgs, OnScheduledTickArgs, PlacedArgs,
};
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
        let entity = BrushableBlockBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(entity));
        // Vanilla `BrushableBlock.onPlace`.
        args.world
            .schedule_block_tick(args.block, *args.position, 2, TickPriority::Normal);
    }

    /// Vanilla `BrushableBlock.tick`: the block entity's `checkReset`.
    /// TODO: falling when the block below is free.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        if let Some(be) = args.world.get_block_entity(args.position)
            && let Some(brushable) = be.as_any().downcast_ref::<BrushableBlockBlockEntity>()
        {
            brushable.check_reset(args.world);
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
