use std::sync::Arc;

use crate::{
    block::{
        BlockBehaviour, CanPlaceAtArgs, GetComparatorOutputArgs, GetStateForNeighborUpdateArgs,
        NormalUseArgs, PathComputationType, UseWithItemArgs,
        blocks::candle_cakes::cake_from_candle, registry::BlockActionResult,
    },
    entity::{EntityBase, player::Player},
    world::World,
};
use pumpkin_data::{
    Block, BlockDirection, BlockState, BlockStateId,
    block_properties::CakeLikeProperties,
    game_event::GameEvent,
    sound::{Sound, SoundCategory},
    tag::{self, Taggable},
};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

/// Vanilla `CakeBlock.getOutputSignal`. Saturates -> an out-of-range bite count reads 0.
#[must_use]
pub const fn cake_output_signal(bites: u8) -> u8 {
    7u8.saturating_sub(bites) * 2
}

/// Vanilla `CakeBlock.FULL_CAKE_SIGNAL`.
pub const FULL_CAKE_SIGNAL: u8 = cake_output_signal(0);

#[pumpkin_block("minecraft:cake")]
pub struct CakeBlock;

impl CakeBlock {
    /// Vanilla `CakeBlock.eat`: one bite of the cake in `state_id` at `location`.
    pub fn eat(
        world: &Arc<World>,
        player: &Player,
        location: &BlockPos,
        state_id: BlockStateId,
    ) -> BlockActionResult {
        if !player.can_eat(false) {
            return BlockActionResult::Pass;
        }
        player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Custom,
            pumpkin_data::statistic::CustomStatistic::EatCakeSlice as i32,
            1,
        );
        player.hunger_manager.eat(player, 2, 0.4);
        let source: Option<&dyn EntityBase> = Some(player);
        world.game_event_from(source, GameEvent::Eat, location.to_centered_f64());

        let mut properties = CakeLikeProperties::from_state_id(state_id);
        if properties.bites < 6 {
            properties.bites += 1;
            world.set_block_state(
                location,
                properties.to_state_id(&Block::CAKE),
                BlockFlags::NOTIFY_ALL,
            );
        } else {
            world.set_block_state(
                location,
                Block::AIR.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
            world.game_event_from(source, GameEvent::BlockDestroy, location.to_centered_f64());
        }
        BlockActionResult::Success
    }
}

impl BlockBehaviour for CakeBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_place_at(args.block_accessor, args.position)
    }

    /// Vanilla `CakeBlock.useItemOn`: a candle goes on an uneaten cake, anything else tries the
    /// empty hand.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        let properties = CakeLikeProperties::from_state_id(state_id);
        let item = args.item_stack.item;
        if !item.has_tag(&tag::Item::MINECRAFT_CANDLES) || properties.bites != 0 {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        let candle_cake = cake_from_candle(item);
        if candle_cake == &Block::CAKE {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        if !args.player.has_infinite_materials() {
            args.item_stack.decrement(1);
        }
        args.world.play_sound(
            Sound::BlockCakeAddCandle,
            SoundCategory::Blocks,
            &args.position.to_centered_f64(),
        );
        args.world.set_block_state(
            args.position,
            candle_cake.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        args.world.game_event_from(
            Some(args.player.as_ref()),
            GameEvent::BlockChange,
            args.position.to_centered_f64(),
        );
        args.player.award_item_used(item);
        BlockActionResult::Success
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        Self::eat(args.world, args.player, args.position, state_id)
    }

    /// Vanilla `CakeBlock.updateShape`: gone once the block below stops holding it.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if args.direction == BlockDirection::Down && !can_place_at(args.world, args.position) {
            return Block::AIR.default_state.id;
        }
        args.state_id
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        let state_id = args.world.get_block_state_id(args.position);
        let properties = CakeLikeProperties::from_state_id(state_id);
        Some(cake_output_signal(properties.bites))
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

fn can_place_at(world: &dyn BlockAccessor, position: &BlockPos) -> bool {
    let state = world.get_block_state(&position.down());
    state.is_solid()
}
