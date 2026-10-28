use crate::item::create_filled_result;
use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::PotionContentsImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Potion;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;

/// A potion holding water; the contents component is what names it and makes it
/// brewable.
#[must_use]
pub fn water_bottle() -> ItemStack {
    ItemStack::new_with_component(
        1,
        &Item::POTION,
        vec![(
            DataComponent::PotionContents,
            Some(Box::new(PotionContentsImpl {
                potion_id: Some(i32::from(Potion::WATER.id)),
                custom_color: None,
                custom_effects: Vec::new(),
                custom_name: None,
            }) as Box<_>),
        )],
    )
}

pub struct GlassBottleItem;

impl ItemMetadata for GlassBottleItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::GLASS_BOTTLE.id])
    }
}

impl ItemBehaviour for GlassBottleItem {
    /// Vanilla `BottleItem.use`.
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        // TODO: filling from the ender dragon's breath clouds.
        let world = player.world();
        let (start_pos, end_pos) = self.get_start_and_end_pos(player);
        let hit = world.clip(&pumpkin_world::clip::ClipContext::new(
            start_pos,
            end_pos,
            pumpkin_world::clip::ClipBlock::Outline,
            pumpkin_world::clip::ClipFluid::SourceOnly,
        ));
        if hit.is_miss {
            return InteractionResult::PASS;
        }
        let hit_pos = hit.block_pos;
        // `level.getFluidState(pos).is(FluidTags.WATER)`: water blocks, waterlogged blocks,
        // kelp, seagrass, bubble columns.
        if !world
            .get_fluid(&hit_pos)
            .has_tag(&pumpkin_data::tag::Fluid::MINECRAFT_WATER)
        {
            return InteractionResult::PASS;
        }
        // `level.playSound(player, player.getX(), ...)`: at the player, for everyone else.
        world.play_sound_expect(
            player,
            Sound::ItemBottleFill,
            SoundCategory::Neutral,
            &player.position(),
        );
        world.game_event_from(
            Some(player),
            pumpkin_data::game_event::GameEvent::FluidPickup,
            hit_pos.to_centered_f64(),
        );
        // `turnBottleIntoItem`.
        player.award_item_used(&Item::GLASS_BOTTLE);
        let mut held = player.inventory().get_stack_in_hand(hand);
        create_filled_result(&mut held, player, water_bottle(), true);
        player.inventory().set_stack_in_hand(hand, held);
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
