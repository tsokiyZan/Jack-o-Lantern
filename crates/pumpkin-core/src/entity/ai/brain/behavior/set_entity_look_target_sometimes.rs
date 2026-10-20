use std::sync::Arc;

use pumpkin_util::random::RandomImpl;

use crate::entity::EntityBase;

use super::super::memory::position_tracker::{EntityTracker, PositionTracker};
use super::super::memory::{MemoryStatus, types};
use super::one_shot::OneShot;

/// Vanilla spends the first sample arming, so the look fires on the second zero.
pub struct Ticker {
    min_interval: i32,
    max_interval: i32,
    ticks_until_next_start: i32,
}

impl Ticker {
    #[must_use]
    pub const fn new(min_interval: i32, max_interval: i32) -> Self {
        Self {
            min_interval,
            max_interval,
            ticks_until_next_start: 0,
        }
    }

    /// Vanilla `Ticker.tickDownAndCheck`: `interval.sample(random) - 1` (`UniformInt`, one draw).
    pub fn tick_down_and_check(&mut self, rng: &mut impl RandomImpl) -> bool {
        if self.ticks_until_next_start == 0 {
            self.ticks_until_next_start =
                rng.next_int_between_inclusive(self.min_interval, self.max_interval) - 1;
            return false;
        }
        self.ticks_until_next_start -= 1;
        self.ticks_until_next_start == 0
    }
}

#[must_use]
pub fn set_entity_look_target_sometimes(
    predicate: impl Fn(&Arc<dyn EntityBase>) -> bool + Send + Sync + 'static,
    max_dist: f32,
    min_interval: i32,
    max_interval: i32,
) -> OneShot {
    let max_dist_squared = f64::from(max_dist * max_dist);
    let mut ticker = Ticker::new(min_interval, max_interval);
    OneShot::new(
        "SetEntityLookTargetSometimes",
        vec![
            (types::LOOK_TARGET.id(), MemoryStatus::ValueAbsent),
            (
                types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
                MemoryStatus::ValuePresent,
            ),
        ],
        move |tick| {
            let body_pos = tick.mob.get_entity().pos.load();
            let target = {
                let ctx = tick.visibility();
                let Some(visible) = ctx.brain.get(types::NEAREST_VISIBLE_LIVING_ENTITIES) else {
                    return false;
                };
                visible.find_closest(&ctx, |entity| {
                    predicate(entity)
                        && entity
                            .get_entity()
                            .pos
                            .load()
                            .squared_distance_to_vec(&body_pos)
                            <= max_dist_squared
                })
            };
            let Some(target) = target else {
                return false;
            };
            // `ticker.tickDownAndCheck(level.getRandom())`.
            let fire = ticker.tick_down_and_check(
                &mut *tick
                    .world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            if !fire {
                return false;
            }
            tick.brain.set(
                types::LOOK_TARGET,
                Arc::new(EntityTracker::new(target, true)) as Arc<dyn PositionTracker>,
            );
            true
        },
    )
}

#[cfg(test)]
mod tests {
    use pumpkin_util::random::legacy_rand::LegacyRand;

    use super::Ticker;

    #[test]
    fn a_fresh_ticker_arms_itself_instead_of_firing() {
        let mut rng = LegacyRand::from_seed(1);
        let mut ticker = Ticker::new(30, 60);
        assert!(!ticker.tick_down_and_check(&mut rng));
    }

    /// Java: `new Random(7).nextInt(31) + 30 - 1` arms the countdown at 34.
    #[test]
    fn the_armed_countdown_matches_java_random() {
        let mut rng = LegacyRand::from_seed(7);
        let mut ticker = Ticker::new(30, 60);
        assert!(!ticker.tick_down_and_check(&mut rng));
        assert_eq!(ticker.ticks_until_next_start, 34);
        for _ in 1..34 {
            assert!(!ticker.tick_down_and_check(&mut rng));
        }
        assert!(ticker.tick_down_and_check(&mut rng));
    }

    #[test]
    fn a_fixed_interval_fires_exactly_on_its_period() {
        let mut rng = LegacyRand::from_seed(1);
        let mut ticker = Ticker::new(3, 3);
        assert!(!ticker.tick_down_and_check(&mut rng));
        assert!(!ticker.tick_down_and_check(&mut rng));
        assert!(ticker.tick_down_and_check(&mut rng));
        assert!(!ticker.tick_down_and_check(&mut rng));
    }
}
