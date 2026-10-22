//! Vanilla `SimpleContainer`'s item moves, over a mob's inventory slots.
//!
//! Vanilla keeps the slots in a `NonNullList` and the container's stack limit at the
//! `Container` default of 99; here the slots are a plain slice of stacks.

use pumpkin_data::item_stack::ItemStack;

/// Vanilla `Container.getMaxStackSize()`'s default, which `SimpleContainer` doesn't override.
const MAX_STACK: u8 = 99;

/// Vanilla `Container.getMaxStackSize(ItemStack)`: the smaller of the container's and the
/// stack's limit.
fn max_stack_size(stack: &ItemStack) -> u8 {
    MAX_STACK.min(stack.get_max_stack_size())
}

/// Vanilla `ItemStack.isSameItemSameComponents`: an empty stack counts as air.
fn is_same_item_same_components(a: &ItemStack, b: &ItemStack) -> bool {
    if !ItemStack::is_same_item(a, b) {
        return false;
    }
    (a.is_empty() && b.is_empty()) || a.are_items_and_components_equal(b)
}

/// Vanilla `SimpleContainer.setItem`: the stack goes in, cut down to the slot's limit.
fn set_item(items: &mut [ItemStack], slot: usize, mut stack: ItemStack) {
    let max = max_stack_size(&stack);
    if stack.item_count > max {
        stack.set_count(max);
    }
    items[slot] = stack;
}

/// Vanilla `SimpleContainer.addItem`: fills the matching stacks first, then the first empty
/// slot, and returns what didn't fit (empty when all of it did).
pub fn add_item(items: &mut [ItemStack], stack: &ItemStack) -> ItemStack {
    if stack.is_empty() {
        return ItemStack::EMPTY.clone();
    }
    let mut remaining = stack.copy_with_count(stack.item_count);
    move_item_to_occupied_slots_with_same_type(items, &mut remaining);
    if remaining.is_empty() {
        return ItemStack::EMPTY.clone();
    }
    move_item_to_empty_slots(items, &mut remaining);
    if remaining.is_empty() {
        ItemStack::EMPTY.clone()
    } else {
        remaining
    }
}

/// Vanilla `SimpleContainer.canAddItem`: an empty slot, or a same-item same-components stack
/// below its own maximum.
#[must_use]
pub fn can_add_item(items: &[ItemStack], stack: &ItemStack) -> bool {
    items.iter().any(|target| {
        target.is_empty()
            || (is_same_item_same_components(target, stack)
                && target.item_count < target.get_max_stack_size())
    })
}

/// Vanilla `SimpleContainer.removeItem` (`ContainerHelper.removeItem`): splits up to `count`
/// off the slot.
pub fn remove_item(items: &mut [ItemStack], slot: usize, count: u8) -> ItemStack {
    let Some(target) = items.get_mut(slot) else {
        return ItemStack::EMPTY.clone();
    };
    if target.is_empty() || count == 0 {
        return ItemStack::EMPTY.clone();
    }
    let removed = target.split(count);
    if target.is_empty() {
        *target = ItemStack::EMPTY.clone();
    }
    removed
}

/// Vanilla `SimpleContainer.fromItemList`: clears the slots, then `addItem`s each stack.
pub fn from_item_list(items: &mut [ItemStack], stacks: impl IntoIterator<Item = ItemStack>) {
    for slot in items.iter_mut() {
        *slot = ItemStack::EMPTY.clone();
    }
    for stack in stacks {
        add_item(items, &stack);
    }
}

/// Vanilla `SimpleContainer.storeAsItemList`: the non-empty slots, in slot order.
pub fn store_as_item_list(items: &[ItemStack]) -> impl Iterator<Item = &ItemStack> {
    items.iter().filter(|stack| !stack.is_empty())
}

/// Vanilla `SimpleContainer.moveItemToEmptySlots`: the whole remainder into the first empty
/// slot (cut to the slot's limit there; the cut-off part is gone, as in vanilla).
fn move_item_to_empty_slots(items: &mut [ItemStack], source: &mut ItemStack) {
    if let Some(slot) = items.iter().position(ItemStack::is_empty) {
        let moved = std::mem::replace(source, ItemStack::EMPTY.clone());
        set_item(items, slot, moved);
    }
}

/// Vanilla `SimpleContainer.moveItemToOccupiedSlotsWithSameType`.
fn move_item_to_occupied_slots_with_same_type(items: &mut [ItemStack], source: &mut ItemStack) {
    for target in items.iter_mut() {
        if is_same_item_same_components(target, source) {
            move_items_between_stacks(source, target);
            if source.is_empty() {
                return;
            }
        }
    }
}

/// Vanilla `SimpleContainer.moveItemsBetweenStacks`.
fn move_items_between_stacks(source: &mut ItemStack, target: &mut ItemStack) {
    let max_count = max_stack_size(target);
    let diff = source
        .item_count
        .min(max_count.saturating_sub(target.item_count));
    if diff > 0 {
        target.increment(diff);
        source.decrement(diff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::item::Item;

    fn slots(n: usize) -> Vec<ItemStack> {
        (0..n).map(|_| ItemStack::EMPTY.clone()).collect()
    }

    #[test]
    fn same_item_merges_before_empty_slots() {
        let mut items = slots(3);
        items[1] = ItemStack::new(60, &Item::BREAD);
        let rest = add_item(&mut items, &ItemStack::new(10, &Item::BREAD));
        assert!(rest.is_empty());
        assert_eq!(items[1].item_count, 64);
        // The remainder goes to the first empty slot, before the matching stack.
        assert_eq!(items[0].item_count, 6);
        assert_eq!(items[0].item.id, Item::BREAD.id);
        assert!(items[2].is_empty());
    }

    #[test]
    fn empty_slot_takes_the_whole_remainder() {
        let mut items = slots(2);
        items[0] = ItemStack::new(1, &Item::WHEAT);
        let rest = add_item(&mut items, &ItemStack::new(20, &Item::BREAD));
        assert!(rest.is_empty());
        assert_eq!(items[1].item_count, 20);
        assert_eq!(items[1].item.id, Item::BREAD.id);
    }

    #[test]
    fn full_container_returns_the_remainder() {
        let mut items = slots(1);
        items[0] = ItemStack::new(63, &Item::BREAD);
        let rest = add_item(&mut items, &ItemStack::new(5, &Item::BREAD));
        assert_eq!(items[0].item_count, 64);
        assert_eq!(rest.item_count, 4);
        assert_eq!(rest.item.id, Item::BREAD.id);
    }

    #[test]
    fn can_add_item_checks_empty_slots_and_room() {
        let mut items = slots(1);
        assert!(can_add_item(&items, &ItemStack::new(1, &Item::BREAD)));
        items[0] = ItemStack::new(63, &Item::BREAD);
        assert!(can_add_item(&items, &ItemStack::new(1, &Item::BREAD)));
        assert!(!can_add_item(&items, &ItemStack::new(1, &Item::WHEAT)));
        items[0] = ItemStack::new(64, &Item::BREAD);
        assert!(!can_add_item(&items, &ItemStack::new(1, &Item::BREAD)));
    }

    #[test]
    fn remove_item_splits_and_empties() {
        let mut items = slots(1);
        items[0] = ItemStack::new(3, &Item::BREAD);
        assert_eq!(remove_item(&mut items, 0, 2).item_count, 2);
        assert_eq!(items[0].item_count, 1);
        assert_eq!(remove_item(&mut items, 0, 5).item_count, 1);
        assert!(items[0].is_empty());
        assert!(remove_item(&mut items, 0, 1).is_empty());
        assert!(remove_item(&mut items, 7, 1).is_empty());
    }

    #[test]
    fn from_item_list_merges_like_add_item() {
        let mut items = slots(2);
        items[1] = ItemStack::new(1, &Item::STONE);
        from_item_list(
            &mut items,
            [
                ItemStack::new(40, &Item::BREAD),
                ItemStack::new(40, &Item::BREAD),
            ],
        );
        assert_eq!(items[0].item_count, 64);
        assert_eq!(items[1].item_count, 16);
        assert_eq!(items[1].item.id, Item::BREAD.id);
    }
}
