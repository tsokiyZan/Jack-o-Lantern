use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};

pub struct SpyglassItem;

impl ItemMetadata for SpyglassItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::SPYGLASS.id])
    }
}

impl ItemBehaviour for SpyglassItem {
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        player.world().play_sound(
            Sound::ItemSpyglassUse,
            SoundCategory::Players,
            &player.position(),
        );
        player.award_item_used(&Item::SPYGLASS);
        // `ItemUtils.startUsingInstantly`.
        let stack = player.inventory().get_stack_in_hand(hand);
        crate::item::start_using_item(player, hand, stack, Self::USE_DURATION);
        InteractionResult::CONSUME
    }

    fn on_stopped_using(&self, _stack: &pumpkin_data::item_stack::ItemStack, player: &Player) {
        player.world().play_sound(
            Sound::ItemSpyglassStopUsing,
            SoundCategory::Players,
            &player.position(),
        );
    }

    fn get_use_duration(&self, _stack: &pumpkin_data::item_stack::ItemStack) -> i32 {
        Self::USE_DURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl SpyglassItem {
    pub const USE_DURATION: i32 = 1200;
}
