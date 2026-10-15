use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
    hash::Hash,
    sync::{Arc, Mutex, MutexGuard, atomic::AtomicI64},
};

use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    chunk::ChunkData,
    tick::{ScheduledTick, TickPriority, scheduler::ChunkTickScheduler},
};

/// Vanilla `LevelTicks`: a level's block or fluid ticks, kept per chunk and drained in game time
/// order.
pub struct LevelTicks<T> {
    container: fn(&ChunkData) -> &ChunkTickScheduler<T>,
    game_time: Arc<AtomicI64>,
    next_tick_for_container: Mutex<FxHashMap<Vector2<i32>, i64>>,
    collect: Mutex<CollectBuffers>,
    run: Mutex<RunState<T>>,
}

/// Buffers reused across ticks so collecting does not allocate.
#[derive(Default)]
struct CollectBuffers {
    due_containers: Vec<Vector2<i32>>,
    containers_to_tick: BinaryHeap<ContainerToTick>,
}

struct RunState<T> {
    to_run_this_tick: VecDeque<ScheduledTick<T>>,
    to_run_this_tick_set: FxHashSet<(BlockPos, T)>,
}

/// A chunk in vanilla's `containersToTick`, ordered by its first tick with
/// `INTRA_TICK_DRAIN_ORDER` (min-first in the max-heap).
struct ContainerToTick {
    priority: TickPriority,
    sub_tick_order: i64,
    pos: Vector2<i32>,
    chunk: Arc<ChunkData>,
}

impl ContainerToTick {
    const fn new<T>(
        pos: Vector2<i32>,
        chunk: Arc<ChunkData>,
        next_tick: &ScheduledTick<T>,
    ) -> Self {
        Self {
            priority: next_tick.priority,
            sub_tick_order: next_tick.sub_tick_order,
            pos,
            chunk,
        }
    }

    fn intra_tick_drain_order<T>(&self, tick: &ScheduledTick<T>) -> Ordering {
        self.priority
            .cmp(&tick.priority)
            .then_with(|| self.sub_tick_order.cmp(&tick.sub_tick_order))
    }
}

impl PartialEq for ContainerToTick {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ContainerToTick {}

impl PartialOrd for ContainerToTick {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ContainerToTick {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.sub_tick_order.cmp(&self.sub_tick_order))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// Lock order: `collect` -> `run` -> a chunk's ticks -> `next_tick_for_container`.
impl<T: Copy + Hash + Eq> LevelTicks<T> {
    #[must_use]
    pub fn new(
        container: fn(&ChunkData) -> &ChunkTickScheduler<T>,
        game_time: Arc<AtomicI64>,
    ) -> Self {
        Self {
            container,
            game_time,
            next_tick_for_container: Mutex::new(FxHashMap::default()),
            collect: Mutex::new(CollectBuffers::default()),
            run: Mutex::new(RunState {
                to_run_this_tick: VecDeque::new(),
                to_run_this_tick_set: FxHashSet::default(),
            }),
        }
    }

    /// Vanilla `LevelTicks.schedule`, for a tick in `chunk`.
    pub fn schedule(&self, chunk: &ChunkData, tick: ScheduledTick<T>) {
        let mut ticks = (self.container)(chunk).lock();
        ticks.set_clock(&self.game_time);
        if ticks.schedule(tick) {
            self.update_container_scheduling(&tick);
        }
    }

    /// Vanilla `LevelChunkTicks.unpack` followed by `LevelTicks.addContainer`: saved ticks start
    /// counting from `current_tick`.
    pub fn start_ticking(&self, chunk: &ChunkData, current_tick: i64) {
        let mut ticks = (self.container)(chunk).lock();
        ticks.set_clock(&self.game_time);
        ticks.unpack(current_tick, |tick| self.update_container_scheduling(tick));
        if let Some(next_tick) = ticks.peek() {
            self.update_container_scheduling(&next_tick);
        }
    }

    /// Vanilla `LevelTicks.tick`: runs every tick due by `current_tick` in chunks passing
    /// `tick_check`, at most `max_ticks_to_process` of them.
    pub fn tick(
        &self,
        current_tick: i64,
        max_ticks_to_process: usize,
        tick_check: impl Fn(&Vector2<i32>) -> bool,
        get_chunk: impl Fn(&Vector2<i32>) -> Option<Arc<ChunkData>>,
        mut output: impl FnMut(BlockPos, T),
    ) {
        self.collect_ticks(current_tick, max_ticks_to_process, tick_check, get_chunk);
        self.run_collected_ticks(&mut output);
        self.cleanup_after_tick();
    }

    fn collect_ticks(
        &self,
        current_tick: i64,
        max_ticks_to_process: usize,
        tick_check: impl Fn(&Vector2<i32>) -> bool,
        get_chunk: impl Fn(&Vector2<i32>) -> Option<Arc<ChunkData>>,
    ) {
        let mut collect = lock(&self.collect);
        let CollectBuffers {
            due_containers,
            containers_to_tick,
        } = &mut *collect;
        let mut run = lock(&self.run);
        let to_run_this_tick = &mut run.to_run_this_tick;

        self.sort_containers_to_tick(
            current_tick,
            due_containers,
            containers_to_tick,
            &tick_check,
            &get_chunk,
        );
        self.drain_containers(
            current_tick,
            max_ticks_to_process,
            containers_to_tick,
            to_run_this_tick,
        );
        // Vanilla `rescheduleLeftoverContainers`.
        for container in containers_to_tick.drain() {
            let next_tick = (self.container)(&container.chunk).lock().peek();
            if let Some(next_tick) = next_tick {
                self.update_container_scheduling(&next_tick);
            }
        }
    }

    fn sort_containers_to_tick(
        &self,
        current_tick: i64,
        due_containers: &mut Vec<Vector2<i32>>,
        containers_to_tick: &mut BinaryHeap<ContainerToTick>,
        tick_check: &impl Fn(&Vector2<i32>) -> bool,
        get_chunk: &impl Fn(&Vector2<i32>) -> Option<Arc<ChunkData>>,
    ) {
        due_containers.extend(
            lock(&self.next_tick_for_container)
                .iter()
                .filter(|(_, next_tick)| **next_tick <= current_tick)
                .map(|(pos, _)| *pos),
        );
        for pos in due_containers.drain(..) {
            let Some(chunk) = get_chunk(&pos) else {
                lock(&self.next_tick_for_container).remove(&pos);
                continue;
            };
            let ticks = (self.container)(&chunk).lock();
            match ticks.peek() {
                None => {
                    lock(&self.next_tick_for_container).remove(&pos);
                }
                Some(next_tick) if next_tick.trigger_tick > current_tick => {
                    lock(&self.next_tick_for_container).insert(pos, next_tick.trigger_tick);
                }
                Some(next_tick) => {
                    if tick_check(&pos) {
                        lock(&self.next_tick_for_container).remove(&pos);
                        drop(ticks);
                        containers_to_tick.push(ContainerToTick::new(pos, chunk, &next_tick));
                    }
                }
            }
        }
    }

    fn drain_containers(
        &self,
        current_tick: i64,
        max_ticks_to_process: usize,
        containers_to_tick: &mut BinaryHeap<ContainerToTick>,
        to_run_this_tick: &mut VecDeque<ScheduledTick<T>>,
    ) {
        while to_run_this_tick.len() < max_ticks_to_process
            && let Some(top_container) = containers_to_tick.pop()
        {
            let mut ticks = (self.container)(&top_container.chunk).lock();
            let Some(tick) = ticks.poll() else {
                continue;
            };
            to_run_this_tick.push_back(tick);

            // Vanilla `drainFromCurrentContainer`.
            if to_run_this_tick.len() < max_ticks_to_process {
                let next_best_container = containers_to_tick.peek();
                while to_run_this_tick.len() < max_ticks_to_process {
                    let Some(next_from_current) = ticks.peek() else {
                        break;
                    };
                    if next_from_current.trigger_tick > current_tick
                        || next_best_container.is_some_and(|next| {
                            next.intra_tick_drain_order(&next_from_current) == Ordering::Less
                        })
                    {
                        break;
                    }
                    ticks.poll();
                    to_run_this_tick.push_back(next_from_current);
                }
            }

            if let Some(next_tick) = ticks.peek() {
                if next_tick.trigger_tick <= current_tick
                    && to_run_this_tick.len() < max_ticks_to_process
                {
                    drop(ticks);
                    containers_to_tick.push(ContainerToTick::new(
                        top_container.pos,
                        top_container.chunk,
                        &next_tick,
                    ));
                } else {
                    self.update_container_scheduling(&next_tick);
                }
            }
        }
    }

    fn update_container_scheduling(&self, next_tick: &ScheduledTick<T>) {
        lock(&self.next_tick_for_container)
            .insert(next_tick.position.chunk_position(), next_tick.trigger_tick);
    }

    fn run_collected_ticks(&self, output: &mut impl FnMut(BlockPos, T)) {
        loop {
            let entry = {
                let mut run = lock(&self.run);
                let Some(entry) = run.to_run_this_tick.pop_front() else {
                    break;
                };
                if !run.to_run_this_tick_set.is_empty() {
                    run.to_run_this_tick_set
                        .remove(&(entry.position, entry.value));
                }
                entry
            };
            output(entry.position, entry.value);
        }
    }

    fn cleanup_after_tick(&self) {
        let mut run = lock(&self.run);
        run.to_run_this_tick.clear();
        run.to_run_this_tick_set.clear();
    }

    /// Vanilla `LevelTicks.willTickThisTick`: whether the tick was collected this tick and has
    /// not run yet.
    pub fn will_tick_this_tick(&self, pos: BlockPos, value: T) -> bool {
        let mut run = lock(&self.run);
        let RunState {
            to_run_this_tick,
            to_run_this_tick_set,
        } = &mut *run;
        if to_run_this_tick_set.is_empty() && !to_run_this_tick.is_empty() {
            to_run_this_tick_set.extend(
                to_run_this_tick
                    .iter()
                    .map(|tick| (tick.position, tick.value)),
            );
        }
        to_run_this_tick_set.contains(&(pos, value))
    }

    /// Vanilla `LevelTicks.clearArea`, for the chunks overlapping the area.
    pub fn clear_area(
        &self,
        chunks: impl IntoIterator<Item = Arc<ChunkData>>,
        is_inside: impl Fn(&BlockPos) -> bool,
    ) {
        for chunk in chunks {
            let mut ticks = (self.container)(&chunk).lock();
            let previous_top = ticks.peek();
            ticks.remove_if(|tick| is_inside(&tick.position));
            let new_top = ticks.peek();
            if new_top.map(|tick| tick.sub_tick_order)
                != previous_top.map(|tick| tick.sub_tick_order)
            {
                match new_top {
                    Some(new_top) => self.update_container_scheduling(&new_top),
                    None => {
                        lock(&self.next_tick_for_container).remove(&Vector2::new(chunk.x, chunk.z));
                    }
                }
            }
        }
        lock(&self.run)
            .to_run_this_tick
            .retain(|tick| !is_inside(&tick.position));
    }
}
