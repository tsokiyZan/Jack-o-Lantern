use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component_impl::RecipesImpl;
use pumpkin_data::item::Item;

pub struct KnowledgeBookItem;
pub struct DiscFragmentItem;

impl ItemMetadata for KnowledgeBookItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::KNOWLEDGE_BOOK.id])
    }
}

impl ItemBehaviour for KnowledgeBookItem {
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let has_recipes = player
            .inventory()
            .get_stack_in_hand(hand)
            .get_data_component::<RecipesImpl>()
            .is_some_and(|recipes| !recipes.recipes.is_empty());
        crate::item::consume_in_hand(player, hand);
        if !has_recipes {
            return InteractionResult::FAIL;
        }
        // TODO: `awardRecipes`; vanilla also fails on a recipe it doesn't know.
        player.award_item_used(&Item::KNOWLEDGE_BOOK);
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ItemMetadata for DiscFragmentItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::DISC_FRAGMENT_5.id])
    }
}

impl ItemBehaviour for DiscFragmentItem {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
