use crate::entity::player::cooldown_group;
use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::{Holder, IdOr, InstrumentImpl, SoundEvent};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::instrument::Instrument;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
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
        let Some(instrument) = Self::instrument(&stack) else {
            return InteractionResult::FAIL;
        };
        let cooldown = cooldown_group(&stack);
        let use_ticks = (instrument.use_duration * 20.0).floor() as i32;
        crate::item::start_using_item(player, hand, stack, use_ticks);
        // `play`: heard by everyone near the player but the player, then the game event.
        let world = player.world();
        world.play_sound_event_from_entity(
            Some(player),
            &player.living_entity.entity,
            &instrument.sound,
            SoundCategory::Records,
            instrument.range / 16.0,
            1.0,
        );
        world.game_event_from(Some(player), GameEvent::InstrumentPlay, player.position());
        if instrument.durability_damage > 0 {
            player.damage_item_in_hand(hand, instrument.durability_damage);
        }
        if instrument.use_duration > 0.0 {
            player.start_cooldown(cooldown, use_ticks);
        }
        player.award_item_used(&Item::GOAT_HORN);
        InteractionResult::CONSUME
    }

    /// Vanilla `InstrumentItem.getUseDuration`: the instrument's, or 0 without one.
    fn get_use_duration(&self, stack: &ItemStack) -> i32 {
        Self::instrument(stack).map_or(0, |instrument| {
            (instrument.use_duration * 20.0).floor() as i32
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// What `InstrumentItem` reads from an instrument.
struct InstrumentValues {
    sound: IdOr<SoundEvent>,
    use_duration: f32,
    range: f32,
    durability_damage: i32,
}

impl GoatHornItem {
    /// Vanilla `InstrumentItem.getInstrument`: the stack's `instrument` component, which a
    /// goat horn has by default (the ponder horn) unless it was removed.
    fn instrument(stack: &ItemStack) -> Option<InstrumentValues> {
        if let Some(component) = stack.get_data_component::<InstrumentImpl>() {
            return Some(match &component.instrument {
                Holder::Reference(instrument) => Self::registry_values(*instrument),
                Holder::Direct(definition) => InstrumentValues {
                    sound: definition.sound_event.clone(),
                    use_duration: definition.use_duration,
                    range: definition.range,
                    durability_damage: definition.durability_damage,
                },
            });
        }
        let removed = stack
            .patch
            .iter()
            .any(|(id, value)| *id == DataComponent::Instrument && value.is_none());
        (!removed).then(|| Self::registry_values(Instrument::PonderGoatHorn))
    }

    const fn registry_values(instrument: Instrument) -> InstrumentValues {
        InstrumentValues {
            sound: IdOr::Id(instrument.sound()),
            use_duration: instrument.use_duration_seconds(),
            range: instrument.range(),
            durability_damage: 0,
        }
    }
}
