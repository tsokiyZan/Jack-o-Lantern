//! Vanilla `ExecutionContext` frames and chain modifiers.
//!
//! Vanilla queues commands and function entries in an `ExecutionContext` held in a thread-local.
//! Pumpkin runs them depth-first on the calling thread, which gives the same order, so the
//! frames live on a thread-local stack here. Discarding a frame stops the work still pending
//! in it: every loop over a frame's entries snapshots [`Frame::generation`] when it starts and
//! stops once it changes, which is what vanilla's `discardAtDepthOrHigher` does to the queue.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::source::{ResultValueTaker, ReturnValue};

/// Vanilla `Frame`: where a function's (or a top-level command's) return value goes.
pub struct Frame {
    pub depth: usize,
    return_value_consumer: ResultValueTaker,
    /// Vanilla `FrameControl`, shared with the parent frame for a `returnParentFrame` call.
    control: Arc<AtomicU64>,
}

impl Frame {
    pub fn return_success(&self, value: i32) {
        self.return_value_consumer.call(ReturnValue::Success(value));
    }

    pub fn return_failure(&self) {
        self.return_value_consumer.call(ReturnValue::Failure);
    }

    /// Drops everything still pending in this frame.
    pub fn discard(&self) {
        self.control.fetch_add(1, Ordering::Relaxed);
    }

    /// Changes whenever this frame is discarded.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.control.load(Ordering::Relaxed)
    }

    #[must_use]
    pub const fn return_value_consumer(&self) -> &ResultValueTaker {
        &self.return_value_consumer
    }
}

thread_local! {
    static FRAMES: RefCell<Vec<Arc<Frame>>> = const { RefCell::new(Vec::new()) };
}

/// The frame of the entry being executed (vanilla `ExecutionControl.currentFrame`).
#[must_use]
pub fn current_frame() -> Option<Arc<Frame>> {
    FRAMES.with_borrow(|frames| frames.last().cloned())
}

struct FrameGuard;

impl Drop for FrameGuard {
    fn drop(&mut self) {
        FRAMES.with_borrow_mut(Vec::pop);
    }
}

/// Runs `body` in a new frame one deeper than the current one, as vanilla `createTopFrame`,
/// `CallFunction` and `IsolatedCall` do. With `return_parent_frame`, discarding the new frame
/// discards the current one too.
pub fn run_in_new_frame<R>(
    return_value_consumer: ResultValueTaker,
    return_parent_frame: bool,
    body: impl FnOnce(&Arc<Frame>) -> R,
) -> R {
    let parent = current_frame();
    let depth = parent.as_ref().map_or(0, |parent| parent.depth + 1);
    let control = match parent {
        Some(parent) if return_parent_frame => parent.control.clone(),
        _ => Arc::new(AtomicU64::new(0)),
    };
    let frame = Arc::new(Frame {
        depth,
        return_value_consumer,
        control,
    });
    FRAMES.with_borrow_mut(|frames| frames.push(frame.clone()));
    let _guard = FrameGuard;
    body(&frame)
}

/// Vanilla `ChainModifiers`.
#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
pub struct ChainModifiers {
    forked: bool,
    is_return: bool,
}

impl ChainModifiers {
    #[must_use]
    pub const fn set_forked(self) -> Self {
        Self {
            forked: true,
            ..self
        }
    }

    #[must_use]
    pub const fn set_return(self) -> Self {
        Self {
            is_return: true,
            ..self
        }
    }

    #[must_use]
    pub const fn is_forked(self) -> bool {
        self.forked
    }

    #[must_use]
    pub const fn is_return(self) -> bool {
        self.is_return
    }
}
