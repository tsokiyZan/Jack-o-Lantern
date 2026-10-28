use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::world::BlockFlags;
use crate::world::game_event::GameEventContext;
use pumpkin_data::block_sound_type::SoundType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::{Block, BlockDirection};

pub struct PlaceOnWaterBlockItem;

impl ItemMetadata for PlaceOnWaterBlockItem {
    fn ids() -> Box<[u16]> {
        [Item::LILY_PAD.id, Item::FROGSPAWN.id].into()
    }
}

impl ItemBehaviour for PlaceOnWaterBlockItem {
    /// Vanilla `PlaceOnWaterBlockItem.use`: `BlockItem.useOn` one block above the hit, which is
    /// `SUCCESS` once placed and `FAIL` otherwise.
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let world = player.world();
        let (start_pos, end_pos) = self.get_start_and_end_pos(player);
        let hit = world.clip(&pumpkin_world::clip::ClipContext::new(
            start_pos,
            end_pos,
            pumpkin_world::clip::ClipBlock::Outline,
            pumpkin_world::clip::ClipFluid::SourceOnly,
        ));
        if hit.is_miss {
            return InteractionResult::FAIL;
        }

        let hit_pos = hit.block_pos;

        let above_pos = hit_pos.up();
        let above_state = world.get_block_state(&above_pos);
        let placed_block = if item.id == Item::LILY_PAD.id {
            &Block::LILY_PAD
        } else {
            &Block::FROGSPAWN
        };
        // Vanilla places through `BlockItem.place`, which checks `canSurvive`.
        if above_state.is_air()
            && world.block_registry.can_place_at(
                None,
                Some(&world),
                world.as_ref(),
                Some(player),
                placed_block,
                placed_block.default_state,
                &above_pos,
                Some(BlockDirection::Up),
                None,
            )
        {
            world.set_block_state(
                &above_pos,
                placed_block.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
            player.trigger_advancement(
                crate::entity::player::advancement::trigger::AdvancementTrigger::PlacedBlock {
                    block_id: format!("minecraft:{}", placed_block.name),
                },
            );
            // Vanilla `BlockItem.place`: the place sound, which the placer predicts, then
            // `BLOCK_PLACE`.
            let placed_state = world.get_block_state(&above_pos);
            let sound_type = SoundType::from_state_id(placed_state.id.as_u16());
            world.play_sound_raw_expect(
                player,
                sound_type.place_sound as u16,
                SoundCategory::Blocks,
                &above_pos.to_centered_f64(),
                f32::midpoint(sound_type.volume, 1.0),
                sound_type.pitch * 0.8,
            );
            world.game_event(
                GameEvent::BlockPlace,
                above_pos.to_centered_f64(),
                GameEventContext::of(Some(player), Some(placed_state)),
            );

            crate::item::consume_in_hand(player, hand);
            return InteractionResult::SUCCESS;
        }
        InteractionResult::FAIL
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
