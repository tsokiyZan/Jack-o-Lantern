//! Vanilla `FrogAi`.

use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};

/// Vanilla `FrogAi.getTemptations`.
#[must_use]
pub fn get_temptations(stack: &ItemStack) -> bool {
    stack.item.has_tag(&tag::Item::MINECRAFT_FROG_FOOD)
}
