use std::sync::Arc;

use crate::block::entities::jukebox::JukeboxBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, EmitsRedstonePowerArgs, GetComparatorOutputArgs, GetRedstonePowerArgs,
    NormalUseArgs, OnStateReplacedArgs, PlacedArgs, UseWithItemArgs,
};
use crate::world::game_event::GameEventContext;
use pumpkin_data::BlockStateId;
use pumpkin_data::block_properties::JukeboxLikeProperties;
use pumpkin_data::data_component_impl::{Holder, JukeboxPlayableImpl};
use pumpkin_data::game_event::GameEvent;
use pumpkin_macros::pumpkin_block;

#[pumpkin_block("minecraft:jukebox")]
pub struct JukeboxBlock;

impl JukeboxBlock {
    fn has_record_state(state_id: BlockStateId) -> bool {
        JukeboxLikeProperties::from_state_id(state_id).has_record
    }
}

impl BlockBehaviour for JukeboxBlock {
    /// Called when the jukebox is placed - creates the block entity
    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        let block_entity = JukeboxBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(block_entity));
    }

    /// Vanilla `JukeboxBlock.useWithoutItem`.
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state(args.position).id;
        if Self::has_record_state(state_id)
            && let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(jukebox) = block_entity.as_any().downcast_ref::<JukeboxBlockEntity>()
        {
            jukebox.pop_out_the_item(args.world);
            return BlockActionResult::Success;
        }
        BlockActionResult::Pass
    }

    /// Vanilla `JukeboxBlock.useItemOn` -> `JukeboxPlayable.tryInsertIntoJukebox`.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let world = args.world;
        let state = world.get_block_state(args.position);
        if Self::has_record_state(state.id) {
            return BlockActionResult::PassToDefaultBlockAction;
        }

        // TODO: play inline (non-registry) songs, which vanilla also accepts.
        let is_registry_song = args
            .item_stack
            .get_data_component::<JukeboxPlayableImpl>()
            .is_some_and(|playable| matches!(playable.song, Holder::Reference(_)));
        if !is_registry_song {
            return BlockActionResult::PassToDefaultBlockAction;
        }

        let inserted = args
            .item_stack
            .split_unless_creative(args.player.gamemode.load(), 1);
        if let Some(block_entity) = world.get_block_entity(args.position)
            && let Some(jukebox) = block_entity.as_any().downcast_ref::<JukeboxBlockEntity>()
        {
            jukebox.set_the_item(world, inserted);
            world.game_event(
                GameEvent::BlockChange,
                args.position.to_centered_f64(),
                GameEventContext::of(Some(args.player.as_ref()), Some(state)),
            );
        }

        args.player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Custom,
            pumpkin_data::statistic::CustomStatistic::PlayRecord as i32,
            1,
        );

        BlockActionResult::Success
    }

    /// Vanilla: `JukeboxBlock.onStateReplaced()` -> `ItemScatterer.onStateReplaced()`
    fn on_state_replaced(&self, _args: OnStateReplacedArgs<'_>) {
        // Vanilla calls ItemScatterer.onStateReplaced which updates comparators
        // TODO: world.updateComparators(pos, block) when implemented
    }

    /// Vanilla: `JukeboxBlock.emitsRedstonePower()` returns true
    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    /// Vanilla: Returns 15 if playing, 0 otherwise
    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        // Vanilla: return world.getBlockEntity(pos) instanceof JukeboxBlockEntity lv && lv.getManager().isPlaying() ? 15 : 0
        if let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(jukebox_entity) = block_entity.as_any().downcast_ref::<JukeboxBlockEntity>()
            && jukebox_entity.is_playing()
        {
            15
        } else {
            0
        }
    }

    /// Vanilla: Returns the song's comparator output (0-15)
    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        // Vanilla: return world.getBlockEntity(pos) instanceof JukeboxBlockEntity lv ? lv.getComparatorOutput() : 0
        if let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(jukebox_entity) = block_entity.as_any().downcast_ref::<JukeboxBlockEntity>()
        {
            let record = jukebox_entity.get_record();
            // Get the song from the record's jukebox_playable component
            if let Some(playable) = record.get_data_component::<JukeboxPlayableImpl>()
                && let Holder::Reference(song) = playable.song
            {
                return Some(song.comparator_output());
            }
        }
        Some(0)
    }
}
