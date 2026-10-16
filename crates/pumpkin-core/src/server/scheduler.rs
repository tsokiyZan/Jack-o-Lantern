use crate::server::Server;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledFunctionEvent {
    pub id: String,
    pub trigger_tick: u64,
    /// Breaks ties between events due on the same tick in scheduling order, like
    /// `TimerQueue.Event.sequentialId`.
    pub sequential_id: u64,
    pub function_name: String,
    pub is_tag: bool,
}

impl PartialOrd for ScheduledFunctionEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledFunctionEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.trigger_tick, other.sequential_id).cmp(&(self.trigger_tick, self.sequential_id))
    }
}

/// Vanilla `TimerQueue` for scheduled functions.
#[derive(Default)]
pub struct ScheduledFunctionQueue {
    queue: Mutex<BinaryHeap<ScheduledFunctionEvent>>,
    sequential_id: AtomicU64,
}

impl ScheduledFunctionQueue {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            queue: Mutex::new(BinaryHeap::new()),
            sequential_id: AtomicU64::new(0),
        }
    }

    pub fn schedule(
        &self,
        id: String,
        trigger_tick: u64,
        function_name: String,
        is_tag: bool,
        replace: bool,
    ) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if replace {
            let mut retained = Vec::new();
            while let Some(event) = queue.pop() {
                if event.id != id {
                    retained.push(event);
                }
            }
            for event in retained {
                queue.push(event);
            }
        }
        // `TimerQueue.schedule` keeps one event per id and time.
        if queue
            .iter()
            .any(|event| event.id == id && event.trigger_tick == trigger_tick)
        {
            return;
        }
        let sequential_id = self.sequential_id.fetch_add(1, AtomicOrdering::Relaxed) + 1;
        queue.push(ScheduledFunctionEvent {
            id,
            trigger_tick,
            sequential_id,
            function_name,
            is_tag,
        });
    }

    pub fn remove(&self, id: &str) -> usize {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut count = 0;
        let mut retained = Vec::new();
        while let Some(event) = queue.pop() {
            if event.id == id {
                count += 1;
            } else {
                retained.push(event);
            }
        }
        for event in retained {
            queue.push(event);
        }
        count
    }

    #[must_use]
    pub fn get_event_ids(&self) -> Vec<String> {
        let queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ids: Vec<String> = queue.iter().map(|e| e.id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// `TimerQueue.tick`: runs due events one at a time, so a function that clears a later one
    /// stops it.
    pub fn tick(&self, server: &Arc<Server>, current_tick: u64) {
        loop {
            let event = {
                let mut queue = self
                    .queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match queue.peek() {
                    Some(event) if event.trigger_tick <= current_tick => queue.pop(),
                    _ => None,
                }
            };
            let Some(event) = event else {
                return;
            };
            let _ = crate::data::datapack::DatapackManager::execute_function_from_game_loop(
                server,
                &event.function_name,
            );
        }
    }
}
