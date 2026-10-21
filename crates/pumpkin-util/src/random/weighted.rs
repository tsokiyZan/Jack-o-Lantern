//! Vanilla `net.minecraft.util.random.WeightedRandom` on a passed random.

use super::RandomImpl;

/// Vanilla `WeightedRandom.getTotalWeight`; panics like vanilla when the sum exceeds `i32::MAX`.
fn get_total_weight<T>(items: &[T], weight: impl Fn(&T) -> i32) -> i32 {
    let total: i64 = items.iter().map(|item| i64::from(weight(item))).sum();
    assert!(
        total <= i64::from(i32::MAX),
        "Sum of weights must be <= 2147483647"
    );
    total as i32
}

/// Vanilla `WeightedRandom.getWeightedItem`.
fn get_weighted_item<T>(items: &[T], mut index: i32, weight: impl Fn(&T) -> i32) -> Option<&T> {
    items.iter().find(|item| {
        index = index.wrapping_sub(weight(item));
        index < 0
    })
}

/// Vanilla `WeightedRandom.getRandomItem(random, items, weightGetter)`: draws `nextInt(total)`
/// unless the total weight is 0.
pub fn get_random_item<'a, T, R: RandomImpl>(
    random: &mut R,
    items: &'a [T],
    weight: impl Fn(&T) -> i32,
) -> Option<&'a T> {
    let total_weight = get_total_weight(items, &weight);
    assert!(total_weight >= 0, "Negative total weight in getRandomItem");
    if total_weight == 0 {
        return None;
    }
    let selection = random.next_bounded_i32(total_weight);
    get_weighted_item(items, selection, weight)
}
