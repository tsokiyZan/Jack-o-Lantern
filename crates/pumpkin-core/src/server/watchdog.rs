//! Server watchdog: forcibly shuts down the process when a single tick exceeds
//! `max_tick_time` milliseconds, mirroring vanilla's `ServerWatchdog`.
//!
//! The ticker writes the monotonic start time of each tick into a shared
//! `AtomicU64`.  The watchdog reads that value, checks whether the server has
//! been inside a tick longer than the allowed limit, and calls `exit(1)` when
//! it has.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::error;

/// Shared state between the ticker (writer) and the watchdog (reader).
///
/// The ticker stores the monotonic start time of the *last* tick here, in
/// milliseconds since [`Instant::now()`].  The watchdog reads it to decide
/// whether a stall has occurred.
pub struct WatchdogState {
    tick_start_ms: AtomicU64,
}

impl Default for WatchdogState {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchdogState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tick_start_ms: AtomicU64::new(0),
        }
    }

    /// Record the instant a tick begins (monotonic millis since an arbitrary epoch).
    pub fn record_tick_start(&self) {
        self.tick_start_ms.store(
            Instant::now().elapsed().as_millis() as u64,
            Ordering::Relaxed,
        );
    }

    /// Read the last recorded tick start time.
    pub fn tick_start_ms(&self) -> u64 {
        self.tick_start_ms.load(Ordering::Relaxed)
    }
}

/// Decide whether the server should be considered stalled.
///
/// Returns `Some(elapsed_seconds)` when the server is inside a tick and the
/// elapsed time exceeds `max_tick_time` milliseconds, mirroring vanilla's
/// `deltaNanos > maxTickTimeNanos` check.  Returns `None` otherwise.
///
/// Pure function, no side effects.
#[must_use]
pub fn stalled(in_tick: bool, tick_start_ms: u64, now_ms: u64, max_tick_time: i64) -> Option<f64> {
    if max_tick_time <= 0 {
        return None;
    }
    if !in_tick {
        return None;
    }
    let elapsed_ms = now_ms.wrapping_sub(tick_start_ms);
    if elapsed_ms > max_tick_time as u64 {
        return Some(elapsed_ms as f64 / 1_000.0);
    }
    None
}

/// Spawn the watchdog thread.
///
/// The watchdog loops while the server is running, sleeping until
/// `tick_start + max_tick_time` each iteration.  When a stall is detected it
/// logs the vanilla messages, dumps thread names, and calls `process::exit(1)`.
///
/// Returns `Err` if the thread could not be spawned (extremely rare).
pub fn spawn(max_tick_time: i64, state: Arc<WatchdogState>) -> std::io::Result<()> {
    if max_tick_time <= 0 {
        return Ok(());
    }

    std::thread::Builder::new()
        .name("Server Watchdog".into())
        .spawn(move || run(max_tick_time, &state))
        .map(|_| ())
}

fn run(max_tick_time: i64, state: &Arc<WatchdogState>) {
    loop {
        let tick_start_ms = state.tick_start_ms();
        let now_ms = Instant::now().elapsed().as_millis() as u64;
        let elapsed_ms = now_ms.wrapping_sub(tick_start_ms);

        if elapsed_ms > max_tick_time as u64 {
            // Stalled — log vanilla messages and exit.
            let elapsed_seconds = elapsed_ms as f64 / 1_000.0;
            error!(
                "A single server tick took {:.2} seconds (should be max 0.05)",
                elapsed_seconds
            );
            error!("Considering it to be crashed, server will forcibly shutdown.");
            thread_dump();
            std::process::exit(1);
        }

        // Sleep until the next possible deadline, never less than 1 ms.
        let deadline = tick_start_ms.saturating_add(max_tick_time as u64);
        let sleep_ms = deadline.saturating_sub(now_ms).max(1);
        std::thread::sleep(Duration::from_millis(sleep_ms));
    }
}

/// Enumerate thread names from `/proc/self/task/*/comm` and log them.
///
/// This is the Linux-equivalent of vanilla's thread dump: `std::thread` does not
/// expose other threads' stacks, but listing comm names is still useful for
/// debugging stalls.
fn thread_dump() {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/proc/self/task") {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(comm) = std::fs::read_to_string(path.join("comm")) {
                let name = comm.trim().to_string();
                names.push(name);
            }
        }
    }
    names.sort();
    for name in names {
        error!("Thread: {name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_when_in_tick_and_exceeds_max() {
        // in_tick = true, elapsed (10001 ms) > max (10000 ms)
        let result = stalled(true, 0, 10_001, 10_000);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), 10.001);
    }

    #[test]
    fn not_stalled_at_exact_max() {
        // elapsed == max → should NOT fire (vanilla uses `>`, not `>=`)
        let result = stalled(true, 0, 10_000, 10_000);
        assert!(result.is_none());
    }

    #[test]
    fn not_stalled_below_max() {
        let result = stalled(true, 0, 5_000, 10_000);
        assert!(result.is_none());
    }

    #[test]
    fn not_stalled_between_ticks() {
        // in_tick = false → must not fire even if elapsed is huge
        let result = stalled(false, 0, 999_999, 10_000);
        assert!(result.is_none());
    }

    #[test]
    fn not_stalled_when_disabled() {
        let result = stalled(true, 0, 999_999, 0);
        assert!(result.is_none());

        let result = stalled(true, 0, 999_999, -1);
        assert!(result.is_none());
    }

    #[test]
    fn stalled_returns_elapsed_seconds() {
        let result = stalled(true, 0, 15_050, 10_000);
        assert_eq!(result, Some(15.05));
    }
}
