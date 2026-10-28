use pumpkin_data::{
    Block, BlockState, BlockStateId, Property,
    block_properties::{DoubleBlockHalf, WaterLikeProperties},
    fluid::Fluid,
    tag::{self, Taggable},
};
use pumpkin_util::{math::position::BlockPos, random::RandomGenerator};

use crate::generation::proto_chunk::GenerationCache;
use crate::{
    generation::block_state_provider::BlockStateProvider,
    world::{BlockAccessor, WorldPortalExt},
};

pub struct SimpleBlockFeature {
    pub to_place: BlockStateProvider,
    pub schedule_tick: Option<bool>,
}

impl SimpleBlockFeature {
    pub fn generate<T: GenerationCache>(
        &self,
        block_registry: &dyn WorldPortalExt,
        chunk: &mut T,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        let state = self.to_place.get(random, pos, chunk, block_registry);
        let block = Block::from_state_id(state.id);
        let block_accessor: &dyn BlockAccessor = chunk;
        if !block_registry.can_place_at(block, state, block_accessor, &pos) {
            return false;
        }

        if is_double_plant(block, state.id) {
            // Vanilla: the space above must be air, or replaceable and holding the same fluid.
            let above = pos.up();
            let above_state = GenerationCache::get_block_state(chunk, &above.0).to_state();
            if !above_state.is_air()
                && (fluid_of(state.id) != fluid_of(above_state.id) || !above_state.replaceable())
            {
                return false;
            }
            place_double_plant(chunk, block, state, pos);
        } else {
            chunk.set_block_state(&pos.0, state);
        }
        true
    }
}

/// Vanilla `instanceof DoublePlantBlock`: the plants (not doors) with a `half` property.
fn is_double_plant(block: &Block, state_id: BlockStateId) -> bool {
    !block.has_tag(&tag::Block::MINECRAFT_DOORS)
        && block.properties(state_id).is_some_and(|props| {
            props
                .to_props()
                .iter()
                .any(|prop| matches!(prop, Property::DoubleBlockHalf(_)))
        })
}

/// `state` with its `half` set.
fn with_half(block: &Block, state: &BlockState, half: DoubleBlockHalf) -> BlockStateId {
    let Some(props) = block.properties(state.id) else {
        return state.id;
    };
    let props: Vec<Property> = props
        .to_props()
        .into_iter()
        .map(|prop| match prop {
            Property::DoubleBlockHalf(_) => Property::DoubleBlockHalf(half),
            other => other,
        })
        .collect();
    block.from_properties(&props).to_state_id(block)
}

/// Vanilla `DoublePlantBlock.placeAt` with `copyWaterloggedFrom`.
fn place_double_plant<T: GenerationCache>(
    chunk: &mut T,
    block: &Block,
    state: &BlockState,
    pos: BlockPos,
) {
    for (at, half) in [
        (pos, DoubleBlockHalf::Lower),
        (pos.up(), DoubleBlockHalf::Upper),
    ] {
        let mut id = with_half(block, state, half);
        let in_water = GenerationCache::get_fluid_and_fluid_state(chunk, &at.0)
            .0
            .matches_type(&Fluid::WATER);
        if let Some(waterlogged) = block.set_waterlogged(id, in_water) {
            id = waterlogged;
        }
        chunk.set_block_state(&at.0, id.to_state());
    }
}

/// `BlockState.getFluidState` as (fluid, level), for comparing two states' fluids.
fn fluid_of(id: BlockStateId) -> (u16, u8) {
    let block = id.to_block();
    if block == &Block::WATER || block == &Block::LAVA {
        let level = WaterLikeProperties::from_state_id(id).level;
        let source = if block == &Block::WATER {
            Fluid::WATER.id
        } else {
            Fluid::LAVA.id
        };
        return (source, level);
    }
    let water = id.is_waterlogged()
        || block == &Block::KELP
        || block == &Block::KELP_PLANT
        || block == &Block::SEAGRASS
        || block == &Block::TALL_SEAGRASS
        || block == &Block::BUBBLE_COLUMN;
    (
        if water {
            Fluid::WATER.id
        } else {
            Fluid::EMPTY.id
        },
        0,
    )
}
