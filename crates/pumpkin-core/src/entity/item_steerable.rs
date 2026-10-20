use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use pumpkin_data::tracked_data::TrackedData;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::random::RandomImpl;

use crate::entity::Entity;

pub trait ItemSteerable: Send + Sync {
    /// Attempts to boost speed. Returns `true` if boost was successfully activated.
    fn boost(&self) -> bool;

    fn as_any(&self) -> &dyn Any;
}

/// Vanilla `ItemBasedSteering`. The boost speeds up the steering client, which reads the boost
/// time from the synced data; the server only keeps a boost from starting while one runs.
#[derive(Default)]
pub struct ItemBasedSteering {
    pub boosting: AtomicBool,
    pub boost_time: AtomicI32,
    /// The synced boost time (`boostTimeAccessor`).
    pub boost_time_total: AtomicI32,
}

impl ItemBasedSteering {
    /// Vanilla `ItemBasedSteering.boost`, with the entity's random (`Pig.boost` /
    /// `Strider.boost` pass `this.getRandom()`).
    pub fn boost(&self, entity: &Entity, boost_time_data: TrackedData) -> bool {
        if self.boosting.load(Ordering::Relaxed) {
            return false;
        }
        self.boosting.store(true, Ordering::Relaxed);
        self.boost_time.store(0, Ordering::Relaxed);
        let total = entity.random().next_bounded_i32(841) + 140;
        self.boost_time_total.store(total, Ordering::Relaxed);
        entity.set_synced_data(boost_time_data, VarInt(total));
        true
    }

    /// Vanilla `ItemBasedSteering.tickBoost`.
    pub fn tick_boost(&self) {
        if self.boosting.load(Ordering::Relaxed)
            && self.boost_time.fetch_add(1, Ordering::Relaxed)
                > self.boost_time_total.load(Ordering::Relaxed)
        {
            self.boosting.store(false, Ordering::Relaxed);
        }
    }
}
