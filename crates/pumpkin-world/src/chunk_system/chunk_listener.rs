use super::ChunkPos;
use crate::level::SyncChunk;
use crossbeam::channel::{Receiver, Sender};
use std::sync::Arc;
use std::sync::{Mutex, Weak};
use tokio::sync::oneshot;

#[expect(clippy::type_complexity)]
pub struct ChunkListener {
    single: Mutex<Vec<(ChunkPos, oneshot::Sender<SyncChunk>)>>,
    global: Mutex<Vec<Sender<(ChunkPos, Weak<crate::chunk::ChunkData>)>>>,
    /// Runs once at the start of the next `add_single_chunk_listener`, before the listener is
    /// in place, so a test can finish a chunk load exactly there.
    #[cfg(test)]
    pub(crate) before_single_register: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Default for ChunkListener {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkListener {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            single: Mutex::new(Vec::new()),
            global: Mutex::new(Vec::new()),
            #[cfg(test)]
            before_single_register: Mutex::new(None),
        }
    }

    pub fn add_single_chunk_listener(&self, pos: ChunkPos) -> oneshot::Receiver<SyncChunk> {
        #[cfg(test)]
        {
            let hook = self
                .before_single_register
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let (tx, rx) = oneshot::channel();
        let mut single = self
            .single
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A waiter that found its chunk loaded after registering drops its receiver.
        single.retain(|(_, send)| !send.is_closed());
        single.push((pos, tx));
        rx
    }

    pub fn add_global_chunk_listener(&self) -> Receiver<(ChunkPos, Weak<crate::chunk::ChunkData>)> {
        let (tx, rx) = crossbeam::channel::unbounded();
        self.global
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(tx);
        rx
    }

    pub fn process_new_chunk(&self, pos: ChunkPos, chunk: &SyncChunk) {
        {
            let mut single = self
                .single
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut i = 0;
            let mut len = single.len();
            while i < len {
                if single[i].0 == pos {
                    let (_, send) = single.remove(i);
                    let _ = send.send(chunk.clone());
                    // log::debug!("single listener {i} send {pos:?}");
                    len -= 1;
                    continue;
                }
                i += 1;
            }
        }
        {
            let weak = Arc::downgrade(chunk);
            let mut global = self
                .global
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut i = 0;
            let mut len = global.len();
            while i < len {
                if matches!(global[i].send((pos, weak.clone())), Ok(())) {
                    // log::debug!("global listener {i} send {pos:?}");
                } else {
                    // log::debug!("one global listener dropped");
                    global.remove(i);
                    len -= 1;
                    continue;
                }
                i += 1;
            }
        }
    }
}
