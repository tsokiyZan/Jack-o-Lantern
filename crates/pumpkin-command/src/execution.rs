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

/// Vanilla `ExecutionContext`'s command quota and fork limit.
struct Limits {
    command_limit: i32,
    fork_limit: i32,
    quota: i32,
    stopped: bool,
}

thread_local! {
    static LIMITS: RefCell<Option<Limits>> = const { RefCell::new(None) };
}

struct LimitsGuard;

impl Drop for LimitsGuard {
    fn drop(&mut self) {
        LIMITS.set(None);
    }
}

/// Runs `body` like vanilla `Commands.executeCommandInContext`: with a new command quota and
/// fork limit, unless a command execution already holds them on this thread.
pub fn execute_in_context<R>(command_limit: i32, fork_limit: i32, body: impl FnOnce() -> R) -> R {
    if LIMITS.with_borrow(Option::is_some) {
        return body();
    }
    let command_limit = command_limit.max(1);
    LIMITS.set(Some(Limits {
        command_limit,
        fork_limit,
        quota: command_limit,
        stopped: false,
    }));
    let _guard = LimitsGuard;
    let result = body();
    // Vanilla runCommandQueue checks the quota before looking for the next entry, so using it
    // up exactly is reported even when nothing was left to run.
    let _ = quota_exhausted();
    result
}

/// Vanilla `ExecutionContext.incrementCost`.
pub fn increment_cost() {
    LIMITS.with_borrow_mut(|limits| {
        if let Some(limits) = limits {
            limits.quota -= 1;
        }
    });
}

/// Whether vanilla `runCommandQueue` stops before its next entry because the command quota ran
/// out. Once it has, nothing else in the execution runs.
#[must_use]
pub fn quota_exhausted() -> bool {
    LIMITS.with_borrow_mut(|limits| match limits {
        Some(limits) if limits.stopped => true,
        Some(limits) if limits.quota <= 0 => {
            limits.stopped = true;
            tracing::info!(
                "Command execution stopped due to limit (executed {} commands)",
                limits.command_limit
            );
            true
        }
        _ => false,
    })
}

/// Vanilla `ExecutionContext.forkLimit`.
#[must_use]
pub fn fork_limit() -> Option<i32> {
    LIMITS.with_borrow(|limits| limits.as_ref().map(|limits| limits.fork_limit))
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
