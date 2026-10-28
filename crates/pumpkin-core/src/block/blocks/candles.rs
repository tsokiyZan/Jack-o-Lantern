use std::sync::Arc;

use pumpkin_data::{
    BlockDirection, BlockStateId,
    block_properties::CandleLikeProperties,
    entity::EntityPose,
    fluid::Fluid,
    game_event::GameEvent,
    sound::{Sound, SoundCategory},
};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockAccessor;
use pumpkin_world::world::BlockFlags;

use crate::block::{GetStateForNeighborUpdateArgs, can_support_center};
use crate::world::World;
use crate::{
    block::{
        BlockIsReplacing,
        registry::BlockActionResult,
        {BlockBehaviour, CanPlaceAtArgs, CanUpdateAtArgs, OnPlaceArgs, UseWithItemArgs},
    },
    entity::EntityBase,
};

/// Vanilla `AbstractCandleBlock.extinguish`: `unlit_state` (the candle or candle cake with
/// `lit` false) with flags 11, the extinguish sound and `BLOCK_CHANGE` from `player`.
pub fn extinguish(
    player: Option<&dyn EntityBase>,
    world: &Arc<World>,
    position: &BlockPos,
    unlit_state: BlockStateId,
) {
    world.set_block_state(position, unlit_state, BlockFlags::NOTIFY_ALL);
    world.play_sound(
        Sound::BlockCandleExtinguish,
        SoundCategory::Blocks,
        &position.to_centered_f64(),
    );
    world.game_event_from(player, GameEvent::BlockChange, position.to_centered_f64());
}

#[pumpkin_block_from_tag("minecraft:candles")]
pub struct CandleBlock;

impl BlockBehaviour for CandleBlock {
    /// Vanilla `CandleBlock.getStateForPlacement`.
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        if let BlockIsReplacing::Itself(state_id) = args.replacing {
            let mut properties = CandleLikeProperties::from_state_id(state_id);
            if properties.candles < 4 {
                properties.candles += 1;
            }
            return properties.to_state_id(args.block);
        }

        let mut properties = CandleLikeProperties::default(args.block);
        properties.waterlogged = args.replacing.water_source();
        properties.to_state_id(args.block)
    }

    /// Vanilla `CandleBlock.useItemOn`: an empty hand that may build puts out a lit candle.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        let mut properties = CandleLikeProperties::from_state_id(state_id);
        if args.item_stack.is_empty() && args.player.may_build() && properties.lit {
            properties.lit = false;
            extinguish(
                Some(args.player.as_ref()),
                args.world,
                args.position,
                properties.to_state_id(args.block),
            );
            return BlockActionResult::Success;
        }
        BlockActionResult::PassToDefaultBlockAction
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_place_at(args.block_accessor, args.position)
    }

    fn can_update_at(&self, args: CanUpdateAtArgs<'_>) -> bool {
        let b = BlockAccessor::get_block(args.world, args.position);
        args.player.get_entity().pose.load() != EntityPose::Crouching
            && CandleLikeProperties::from_state_id(args.state_id).candles < 4
            && args.block.id == b.id
    }

    /// Vanilla `CandleBlock.updateShape`: only the water tick. Candles do not break when their
    /// support goes.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let properties = CandleLikeProperties::from_state_id(args.state_id);
        if properties.waterlogged {
            args.world.schedule_fluid_tick(
                &Fluid::WATER,
                *args.position,
                Fluid::WATER.flow_speed as i32,
                TickPriority::Normal,
            );
        }
        args.state_id
    }
}

/// Vanilla `CandleBlock.canSurvive`.
fn can_place_at(block_accessor: &dyn BlockAccessor, position: &BlockPos) -> bool {
    can_support_center(block_accessor, &position.down(), BlockDirection::Up)
}
