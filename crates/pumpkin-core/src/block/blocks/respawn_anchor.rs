use pumpkin_data::block_properties::RespawnAnchorLikeProperties;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};
use std::sync::Arc;

use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId, translation};
use pumpkin_util::math::position::BlockPos;

use crate::world::{
    DefaultExplosionDamageCalculator, Explosion, ExplosionDamageCalculator, ExplosionDamageSource,
    ExplosionEffects, ExplosionInteraction, World,
};
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::BlockFlags;

use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, GetComparatorOutputArgs, NormalUseArgs, PathComputationType, UseWithItemArgs,
};
use crate::world::game_event::GameEventContext;
use pumpkin_data::game_event::GameEvent;
use pumpkin_inventory::screen_handler::InventoryPlayer;

/// Vanilla `RespawnAnchorBlock.MAX_CHARGES`.
const MAX_CHARGES: u8 = 4;

#[pumpkin_block("minecraft:respawn_anchor")]
pub struct RespawnAnchorBlock;

impl BlockBehaviour for RespawnAnchorBlock {
    /// Vanilla `RespawnAnchorBlock.useItemOn`: glowstone charges an anchor below four charges.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        let mut props = RespawnAnchorLikeProperties::from_state_id(state_id);
        let can_be_charged = props.charges < MAX_CHARGES;

        if args.item_stack.item == &Item::GLOWSTONE && can_be_charged {
            // `charge`.
            props.charges += 1;
            let new_state_id = props.to_state_id(args.block);
            args.world
                .set_block_state(args.position, new_state_id, BlockFlags::NOTIFY_ALL);
            args.world.game_event(
                GameEvent::BlockChange,
                args.position.to_centered_f64(),
                GameEventContext::of(
                    Some(args.player.as_ref()),
                    Some(BlockState::from_id(new_state_id)),
                ),
            );
            args.world.play_sound(
                Sound::BlockRespawnAnchorCharge,
                SoundCategory::Blocks,
                &args.position.to_centered_f64(),
            );
            if !args.player.has_infinite_materials() {
                args.item_stack.decrement(1);
            }
            return BlockActionResult::Success;
        }

        // The off hand's glowstone gets its turn instead of the empty-hand use.
        if matches!(args.equipment_slot, EquipmentSlot::MainHand(_))
            && can_be_charged
            && args.player.inventory().off_hand_item().item == &Item::GLOWSTONE
        {
            return BlockActionResult::Pass;
        }
        BlockActionResult::PassToDefaultBlockAction
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        let props = RespawnAnchorLikeProperties::from_state_id(state_id);

        // Vanilla `useWithoutItem`: an empty anchor does nothing, anywhere.
        if props.charges == 0 {
            return BlockActionResult::Pass;
        }

        if !args.world.dimension.respawn_anchor_works {
            Self::explode(args.world, args.position);
            return BlockActionResult::SuccessServer;
        }

        let player = args.player;
        let world = args.world;
        let pos = *args.position;
        // `RespawnData.of(dimension, pos, 0.0F, 0.0F)`.
        if player.set_respawn_point(world.dimension.clone(), pos, 0.0, 0.0, false) {
            world.play_sound(
                Sound::BlockRespawnAnchorSetSpawn,
                SoundCategory::Blocks,
                &pos.to_centered_f64(),
            );

            player.send_system_message(&pumpkin_macros::translate_cross!(
                translation::java::BLOCK_MINECRAFT_SET_SPAWN,
                translation::bedrock::TILE_BED_RESPAWNSET
            ));
            return BlockActionResult::SuccessServer;
        }

        // Already this anchor (or a plugin cancelled it).
        BlockActionResult::Consume
    }

    /// Charges scale over the full signal range, so each charge is worth 15 / 4.
    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        let props = RespawnAnchorLikeProperties::from_state_id(args.state.id);
        Some(props.charges * 15 / MAX_CHARGES)
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

impl RespawnAnchorBlock {
    /// Vanilla `RespawnAnchorBlock.isWaterThatWouldFlow`.
    fn is_water_that_would_flow(world: &World, pos: &BlockPos) -> bool {
        let (fluid, state) = world.get_fluid_and_fluid_state(pos);
        if !fluid.has_tag(&tag::Fluid::MINECRAFT_WATER) {
            return false;
        }
        if state.is_source {
            return true;
        }
        if state.level < 2 {
            return false;
        }
        let (below, _) = world.get_fluid_and_fluid_state(&pos.down());
        !below.has_tag(&tag::Fluid::MINECRAFT_WATER)
    }

    /// Vanilla `RespawnAnchorBlock.explode`: removed, then a fiery `badRespawnPointExplosion`
    /// whose own block counts as water when water would flow in.
    fn explode(world: &Arc<World>, pos: &BlockPos) {
        world.set_block_state(pos, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
        let any_water_neighbors = [
            BlockDirection::North,
            BlockDirection::East,
            BlockDirection::South,
            BlockDirection::West,
        ]
        .iter()
        .any(|direction| Self::is_water_that_would_flow(world, &pos.offset(direction.to_offset())));
        let in_water = any_water_neighbors
            || world
                .get_fluid_and_fluid_state(&pos.up())
                .0
                .has_tag(&tag::Fluid::MINECRAFT_WATER);
        let calculator = Arc::new(AnchorExplosionCalculator {
            pos: *pos,
            in_water,
        });
        let boom_pos = pos.to_centered_f64();
        world.explode_full(
            None,
            Some(ExplosionDamageSource::bad_respawn_point(boom_pos)),
            Some(calculator),
            boom_pos,
            5.0,
            true,
            ExplosionInteraction::Block,
            ExplosionEffects::DEFAULT,
        );
    }
}

/// The anonymous calculator in vanilla `RespawnAnchorBlock.explode`.
struct AnchorExplosionCalculator {
    pos: BlockPos,
    in_water: bool,
}

impl ExplosionDamageCalculator for AnchorExplosionCalculator {
    fn get_block_explosion_resistance(
        &self,
        explosion: &Explosion,
        world: &World,
        pos: &BlockPos,
        block: &Block,
        state: &BlockState,
        fluid: &pumpkin_data::fluid::FluidState,
    ) -> Option<f32> {
        if *pos == self.pos && self.in_water {
            Some(Block::WATER.blast_resistance)
        } else {
            DefaultExplosionDamageCalculator
                .get_block_explosion_resistance(explosion, world, pos, block, state, fluid)
        }
    }
}
