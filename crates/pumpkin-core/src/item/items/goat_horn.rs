use crate::entity::player::cooldown_group;
use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component_impl::CustomDataImpl;
use pumpkin_data::instrument::Instrument;
use pumpkin_data::item::Item;
use pumpkin_data::sound::SoundCategory;

pub struct GoatHornItem;

impl ItemMetadata for GoatHornItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::GOAT_HORN.id])
    }
}

impl ItemBehaviour for GoatHornItem {
    /// Vanilla `InstrumentItem.use`.
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let stack = player.inventory().get_stack_in_hand(hand);
        let instrument = stack
            .get_data_component::<CustomDataImpl>()
            .and_then(|custom| {
                custom
                    .data
                    .get_string("instrument")
                    .or_else(|| custom.data.get_string("Instrument"))
            })
            .and_then(Instrument::from_name)
            .unwrap_or(Instrument::PonderGoatHorn);

        player.world().play_sound(
            instrument.sound(),
            SoundCategory::Players,
            &player.position(),
        );
        let cooldown = cooldown_group(&stack);
        crate::item::start_using_item(player, hand, stack, instrument.use_duration_ticks() as i32);
        // `addCooldown(itemStack, floor(useDuration * 20))`.
        player.start_cooldown(cooldown, instrument.use_duration_ticks() as i32);
        player.award_item_used(&Item::GOAT_HORN);
        InteractionResult::CONSUME
    }

    fn get_use_duration(&self) -> i32 {
        Instrument::PonderGoatHorn.use_duration_ticks() as i32
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl GoatHornItem {
    pub const USE_DURATION: i32 = Instrument::PonderGoatHorn.use_duration_ticks() as i32;
}
