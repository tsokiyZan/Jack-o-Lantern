use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::Block;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::PotionContentsImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Potion;
use pumpkin_data::sound::{Sound, SoundCategory};

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
    fn normal_use(&self, _item: &Item, player: &Player) {
        let world = player.world();
        let (start_pos, end_pos) = self.get_start_and_end_pos(player);
        let hit = world.clip(&pumpkin_world::clip::ClipContext::new(
            start_pos,
            end_pos,
            pumpkin_world::clip::ClipBlock::Outline,
            pumpkin_world::clip::ClipFluid::SourceOnly,
        ));

        if !hit.is_miss {
            let hit_pos = hit.block_pos;
            let (block, state_id) = world.get_block_and_state_id(&hit_pos);
            if block.id == Block::WATER.id || block.is_waterlogged(state_id) {
                world.play_sound(
                    Sound::ItemBottleFill,
                    SoundCategory::Players,
                    &hit_pos.to_f64(),
                );

                let water_bottle = water_bottle();
                let mut held = player.inventory().held_item();
                let mut is_main = true;
                if held.is_empty() || held.item.id != Item::GLASS_BOTTLE.id {
                    held = player.inventory().off_hand_item();
                    is_main = false;
                    if held.is_empty() || held.item.id != Item::GLASS_BOTTLE.id {
                        return;
                    }
                }

                // `turnBottleIntoItem`.
                player.award_item_used(&Item::GLASS_BOTTLE);
                if held.item_count == 1
                    && player.gamemode.load() != pumpkin_util::GameMode::Creative
                {
                    if is_main {
                        player.inventory().set_held_item(water_bottle);
                    } else {
                        player
                            .inventory()
                            .set_stack_in_hand(pumpkin_util::Hand::Left, water_bottle);
                    }
                } else {
                    held.decrement_unless_creative(player.gamemode.load(), 1);
                    if is_main {
                        player.inventory().set_held_item(held);
                    } else {
                        player
                            .inventory()
                            .set_stack_in_hand(pumpkin_util::Hand::Left, held);
                    }
                    let mut stack_to_give = water_bottle;
                    let was_added = player.inventory().insert_stack_anywhere(&mut stack_to_give);
                    if !was_added && !stack_to_give.is_empty() {
                        world.drop_stack(&player.position().to_block_pos(), stack_to_give);
                    }
                }
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
