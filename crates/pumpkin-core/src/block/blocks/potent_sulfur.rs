use pumpkin_data::{
    Block, BlockState, BlockStateId,
    block_properties::{PotentSulfurLikeProperties, PotentSulfurState},
    fluid::Fluid,
    game_event::GameEvent,
    sound::{Sound, SoundCategory},
    tag::{self, Taggable},
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;

use crate::{
    block::{
        BlockBehaviour, GetStateForNeighborUpdateArgs, OnPlaceArgs, OnSyncedBlockEventArgs,
        PlacedArgs, entities::potent_sulfur::PotentSulfurBlockEntity,
    },
    world::{World, game_event::GameEventContext},
};

#[pumpkin_block("minecraft:potent_sulfur")]
pub struct PotentSulfurBlock;

impl PotentSulfurBlock {
    /// Vanilla `FluidState.isSourceOfType(Fluids.WATER)`.
    #[must_use]
    pub fn is_water_source(world: &World, pos: &BlockPos) -> bool {
        let (fluid, fluid_state) = world.get_fluid_and_fluid_state(pos);
        fluid_state.is_source && fluid.matches_type(&Fluid::WATER)
    }

    /// Vanilla `PotentSulfurBlock.validBlockState`.
    fn valid_block_state(world: &World, pos: &BlockPos, state_id: BlockStateId) -> BlockStateId {
        let mut props = PotentSulfurLikeProperties::from_state_id(state_id);
        let current = props.potent_sulfur_state;
        props.potent_sulfur_state = Self::valid_state(world, pos, current);
        props.to_state_id(&Block::POTENT_SULFUR)
    }

    fn valid_state(world: &World, pos: &BlockPos, current: PotentSulfurState) -> PotentSulfurState {
        if !Self::is_water_source(world, &pos.up()) {
            return PotentSulfurState::Dry;
        }
        let below_pos = pos.down();
        let below_block = world.get_block(&below_pos);
        // `isSourceIfFluid`.
        let (_, below_fluid) = world.get_fluid_and_fluid_state(&below_pos);
        let source_if_fluid = below_fluid.is_empty || below_fluid.is_source;
        if below_block.has_tag(&tag::Block::MINECRAFT_CAUSES_CONTINUOUS_GEYSER_ERUPTIONS)
            && source_if_fluid
        {
            return PotentSulfurState::Continuous;
        }
        if below_block.has_tag(&tag::Block::MINECRAFT_CAUSES_PERIODIC_GEYSER_ERUPTIONS)
            && source_if_fluid
        {
            let is_geyser = matches!(
                current,
                PotentSulfurState::Erupting | PotentSulfurState::Dormant
            );
            if !is_geyser
                && let Some(entity) = world.get_block_entity(pos)
                && let Some(sulfur) = entity.as_any().downcast_ref::<PotentSulfurBlockEntity>()
            {
                sulfur.reset_countdown();
            }
            return if current == PotentSulfurState::Erupting {
                PotentSulfurState::Erupting
            } else {
                PotentSulfurState::Dormant
            };
        }
        PotentSulfurState::Wet
    }
}

impl BlockBehaviour for PotentSulfurBlock {
    /// Vanilla `PotentSulfurBlock.getStateForPlacement`.
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        Self::valid_block_state(args.world, args.position, args.block.default_state.id)
    }

    /// Vanilla `PotentSulfurBlock.onPlace`: an erupting or continuous geyser starts with block
    /// event 0, its start sound and `BLOCK_ACTIVATE`, on every state change into it.
    fn placed(&self, args: PlacedArgs<'_>) {
        let state = PotentSulfurLikeProperties::from_state_id(args.state_id).potent_sulfur_state;
        let sound = match state {
            PotentSulfurState::Erupting => Sound::BlockPotentSulfurGeyserEruption,
            PotentSulfurState::Continuous => Sound::BlockPotentSulfurGeyserContinuousEruption,
            PotentSulfurState::Dry | PotentSulfurState::Wet | PotentSulfurState::Dormant => return,
        };
        let pos = *args.position;
        args.world.add_synced_block_event(pos, 0, 0);
        args.world
            .play_sound(sound, SoundCategory::Blocks, &pos.to_centered_f64());
        args.world.game_event(
            GameEvent::BlockActivate,
            pos.to_centered_f64(),
            GameEventContext::of_state(BlockState::from_id(args.state_id)),
        );
    }

    /// Vanilla `PotentSulfurBlock.updateShape`: every neighbour change revalidates.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        Self::valid_block_state(args.world, args.position, args.state_id)
    }

    /// Vanilla `PotentSulfurBlock.triggerEvent`: the eruption's start tick.
    fn on_synced_block_event(&self, args: OnSyncedBlockEventArgs<'_>) -> bool {
        if let Some(entity) = args.world.get_block_entity(args.position)
            && let Some(sulfur) = entity.as_any().downcast_ref::<PotentSulfurBlockEntity>()
        {
            sulfur.set_eruption_tick(args.world.get_world_age());
        }
        true
    }
}
