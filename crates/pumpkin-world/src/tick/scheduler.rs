use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    hash::Hash,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicI64, Ordering as AtomicOrdering},
    },
};

use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashSet;

use crate::tick::{SavedTick, ScheduledTick};

/// Vanilla `LevelChunkTicks`: one chunk's block or fluid ticks.
pub struct ChunkTickScheduler<T> {
    inner: Mutex<ChunkTicks<T>>,
}

pub(crate) struct ChunkTicks<T> {
    tick_queue: BinaryHeap<DrainOrder<T>>,
    /// Ticks read from disk or generation, kept relative until the chunk starts ticking.
    pending_ticks: Option<Vec<SavedTick<T>>>,
    ticks_per_position: FxHashSet<(BlockPos, T)>,
    /// Game time of the level that scheduled or unpacked the queued ticks, used to save them
    /// relative to it.
    clock: Option<Arc<AtomicI64>>,
}

/// Orders the max-heap so it pops vanilla's `ScheduledTick.DRAIN_ORDER` minimum first.
struct DrainOrder<T>(ScheduledTick<T>);

impl<T> PartialEq for DrainOrder<T> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl<T> Eq for DrainOrder<T> {}

impl<T> PartialOrd for DrainOrder<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for DrainOrder<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.drain_order(&self.0)
    }
}

impl<T> ChunkTickScheduler<T> {
    pub(crate) fn lock(&self) -> MutexGuard<'_, ChunkTicks<T>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<T: Copy + Hash + Eq> ChunkTickScheduler<T> {
    /// Vanilla `LevelChunkTicks(List<SavedTick>)`. Duplicate positions keep their first tick,
    /// as vanilla `ProtoChunkTicks` does when they are scheduled.
    #[must_use]
    pub fn from_saved(saved_ticks: Vec<SavedTick<T>>) -> Self {
        let mut ticks_per_position = FxHashSet::default();
        let mut pending_ticks = saved_ticks;
        pending_ticks.retain(|tick| ticks_per_position.insert((tick.position, tick.value)));
        Self {
            inner: Mutex::new(ChunkTicks {
                tick_queue: BinaryHeap::new(),
                pending_ticks: (!pending_ticks.is_empty()).then_some(pending_ticks),
                ticks_per_position,
                clock: None,
            }),
        }
    }

    /// Vanilla `LevelChunkTicks.hasScheduledTick`.
    pub fn has_scheduled_tick(&self, pos: BlockPos, value: T) -> bool {
        self.lock().ticks_per_position.contains(&(pos, value))
    }

    /// Vanilla `LevelChunkTicks.pack`, against the game time of the level the ticks belong to.
    #[must_use]
    pub fn to_saved_ticks(&self) -> Vec<SavedTick<T>> {
        let inner = self.lock();
        let current_tick = inner
            .clock
            .as_ref()
            .map_or(0, |clock| clock.load(AtomicOrdering::Relaxed));
        let mut ticks = Vec::with_capacity(inner.count());
        if let Some(pending_ticks) = &inner.pending_ticks {
            ticks.extend_from_slice(pending_ticks);
        }
        let mut sorted_ticks: Vec<_> = inner.tick_queue.iter().map(|tick| tick.0).collect();
        sorted_ticks.sort_unstable_by_key(|tick| tick.sub_tick_order);
        ticks.extend(
            sorted_ticks
                .iter()
                .map(|tick| tick.to_saved_tick(current_tick)),
        );
        ticks
    }
}

impl<T: Copy + Hash + Eq> ChunkTicks<T> {
    pub(crate) fn peek(&self) -> Option<ScheduledTick<T>> {
        self.tick_queue.peek().map(|tick| tick.0)
    }

    pub(crate) fn poll(&mut self) -> Option<ScheduledTick<T>> {
        let result = self.tick_queue.pop()?.0;
        self.ticks_per_position
            .remove(&(result.position, result.value));
        Some(result)
    }

    /// Vanilla `LevelChunkTicks.schedule`. Returns whether the tick is now the first to run,
    /// which is when vanilla's `onTickAdded` updates the level's schedule.
    pub(crate) fn schedule(&mut self, tick: ScheduledTick<T>) -> bool {
        self.ticks_per_position.insert((tick.position, tick.value)) && self.schedule_unchecked(tick)
    }

    fn schedule_unchecked(&mut self, tick: ScheduledTick<T>) -> bool {
        self.tick_queue.push(DrainOrder(tick));
        self.peek().is_some_and(|head| {
            head.sub_tick_order == tick.sub_tick_order
                && head.trigger_tick == tick.trigger_tick
                && head.priority == tick.priority
                && head.position == tick.position
                && head.value == tick.value
        })
    }

    /// Vanilla `LevelChunkTicks.unpack`. `on_head` runs for each tick that becomes the first to
    /// run.
    pub(crate) fn unpack(&mut self, current_tick: i64, mut on_head: impl FnMut(&ScheduledTick<T>)) {
        if let Some(pending_ticks) = self.pending_ticks.take() {
            let sub_tick_base = -(pending_ticks.len() as i64);
            for (sub_tick_order, pending_tick) in (sub_tick_base..).zip(pending_ticks) {
                let tick = pending_tick.unpack(current_tick, sub_tick_order);
                if self.schedule_unchecked(tick) {
                    on_head(&tick);
                }
            }
        }
    }

    /// Vanilla `LevelChunkTicks.removeIf`.
    pub(crate) fn remove_if(&mut self, mut test: impl FnMut(&ScheduledTick<T>) -> bool) {
        let ticks_per_position = &mut self.ticks_per_position;
        self.tick_queue.retain(|tick| {
            if test(&tick.0) {
                ticks_per_position.remove(&(tick.0.position, tick.0.value));
                false
            } else {
                true
            }
        });
    }

    pub(crate) fn set_clock(&mut self, clock: &Arc<AtomicI64>) {
        if self.clock.is_none() {
            self.clock = Some(clock.clone());
        }
    }

    fn count(&self) -> usize {
        self.tick_queue.len() + self.pending_ticks.as_ref().map_or(0, Vec::len)
    }
}

impl<T> Default for ChunkTickScheduler<T> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(ChunkTicks {
                tick_queue: BinaryHeap::new(),
                pending_ticks: None,
                ticks_per_position: FxHashSet::default(),
                clock: None,
            }),
        }
    }
}
