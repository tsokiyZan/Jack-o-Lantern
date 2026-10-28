use pumpkin_data::{
    Block, BlockDirection, BlockState, BlockStateId, block_properties::CandleCakeProperties,
    item::Item,
};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockAccessor;

use crate::{
    block::{
        BlockBehaviour, CanPlaceAtArgs, GetComparatorOutputArgs, GetStateForNeighborUpdateArgs,
        NormalUseArgs, PathComputationType, UseWithItemArgs,
        blocks::{
            cake::{CakeBlock, FULL_CAKE_SIGNAL},
            candles::extinguish,
        },
        drop_loot,
        registry::BlockActionResult,
    },
    world::loot::LootContextParameters,
};

const CANDLE_MAP: [(&Item, &Block); 17] = [
    (&Item::CANDLE, &Block::CANDLE_CAKE),
    (&Item::WHITE_CANDLE, &Block::WHITE_CANDLE_CAKE),
    (&Item::ORANGE_CANDLE, &Block::ORANGE_CANDLE_CAKE),
    (&Item::MAGENTA_CANDLE, &Block::MAGENTA_CANDLE_CAKE),
    (&Item::LIGHT_BLUE_CANDLE, &Block::LIGHT_BLUE_CANDLE_CAKE),
    (&Item::YELLOW_CANDLE, &Block::YELLOW_CANDLE_CAKE),
    (&Item::LIME_CANDLE, &Block::LIME_CANDLE_CAKE),
    (&Item::PINK_CANDLE, &Block::PINK_CANDLE_CAKE),
    (&Item::GRAY_CANDLE, &Block::GRAY_CANDLE_CAKE),
    (&Item::LIGHT_GRAY_CANDLE, &Block::LIGHT_GRAY_CANDLE_CAKE),
    (&Item::CYAN_CANDLE, &Block::CYAN_CANDLE_CAKE),
    (&Item::PURPLE_CANDLE, &Block::PURPLE_CANDLE_CAKE),
    (&Item::BLUE_CANDLE, &Block::BLUE_CANDLE_CAKE),
    (&Item::BROWN_CANDLE, &Block::BROWN_CANDLE_CAKE),
    (&Item::GREEN_CANDLE, &Block::GREEN_CANDLE_CAKE),
    (&Item::RED_CANDLE, &Block::RED_CANDLE_CAKE),
    (&Item::BLACK_CANDLE, &Block::BLACK_CANDLE_CAKE),
];

#[must_use]
pub fn cake_from_candle(item: &Item) -> &'static Block {
    CANDLE_MAP
        .binary_search_by_key(&item.id, |(key, _)| key.id)
        .map_or(&Block::CAKE, |index| CANDLE_MAP[index].1)
}

#[must_use]
pub fn candle_from_cake(block: &Block) -> &'static Item {
    CANDLE_MAP
        .binary_search_by_key(&block.id, |(_, value)| value.id)
        .map_or(&Item::CANDLE, |index| CANDLE_MAP[index].0)
}

#[pumpkin_block_from_tag("minecraft:candle_cakes")]
pub struct CandleCakeBlock;

impl BlockBehaviour for CandleCakeBlock {
    /// Vanilla `CandleCakeBlock.useItemOn`: an empty hand on the candle puts it out.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let item = args.item_stack.item;
        if item == &Item::FLINT_AND_STEEL || item == &Item::FIRE_CHARGE {
            return BlockActionResult::Pass;
        }
        let state_id = args.world.get_block_state_id(args.position);
        let mut properties = CandleCakeProperties::from_state_id(state_id);
        // `candleHit`: above the cake's top half.
        if args.hit.cursor_pos.y > 0.5 && args.item_stack.is_empty() && properties.lit {
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

    /// Vanilla `CandleCakeBlock.useWithoutItem`: a bite of a whole cake, then the candle cake's
    /// drops (`dropResources`).
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        let result = CakeBlock::eat(
            args.world,
            args.player,
            args.position,
            Block::CAKE.default_state.id,
        );
        if result.consumes_action() {
            let params = LootContextParameters {
                block_state: Some(BlockState::from_id(state_id)),
                position: Some(args.position.to_centered_f64()),
                ..Default::default()
            };
            drop_loot(args.world, args.block, args.position, false, &params);
        }
        result
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_place_at(args.block_accessor, args.position)
    }

    /// Vanilla `CandleCakeBlock.updateShape`: gone once the block below stops holding it.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if args.direction == BlockDirection::Down && !can_place_at(args.world, args.position) {
            return Block::AIR.default_state.id;
        }
        args.state_id
    }

    /// A candle cake is always uneaten, so it reads a full cake.
    fn get_comparator_output(&self, _args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        Some(FULL_CAKE_SIGNAL)
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

/// Vanilla `CandleCakeBlock.canSurvive`.
fn can_place_at(world: &dyn BlockAccessor, position: &BlockPos) -> bool {
    let state = world.get_block_state(&position.down());
    state.is_solid()
}
