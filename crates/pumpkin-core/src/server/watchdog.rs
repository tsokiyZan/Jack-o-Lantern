//! Server watchdog: forcibly shuts down the process when a single tick exceeds
//! `max_tick_time` milliseconds, mirroring vanilla's `ServerWatchdog`.
//!
//! The ticker calls [`WatchdogState::begin_tick`] at the start of each tick,
//! storing the monotonic start time and an `in_tick` flag.  The watchdog
//! reads those values, checks whether the server has been inside a tick
//! longer than the allowed limit, and calls `process::exit(1)` when it has.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
use tracing::error;

/// Shared state between the ticker (writer) and the watchdog (reader).
///
/// The ticker stores the monotonic start time of the *last* tick here, in
/// nanoseconds since a single shared [`Instant`].  The watchdog reads it to
/// decide whether a stall has occurred.
pub struct WatchdogState {
    /// Single monotonic base set once in [`Self::new`].  All elapsed times are
    /// measured from this instant, so the ticker and watchdog share the same
    /// clock.
    base: Instant,
    /// Nanoseconds since `base` when the last tick started.
    tick_start_nanos: AtomicU64,
    /// Whether we are currently inside a tick.
    in_tick: AtomicBool,
    /// Maximum allowed tick time in nanoseconds (derived from `max_tick_time_ms`).
    max_tick_time_nanos: u64,
}

impl Default for WatchdogState {
    fn default() -> Self {
        Self::new(0)
    }
}

impl WatchdogState {
    /// Create a new watchdog state with the given maximum tick time in
    /// milliseconds.  Pass `0` to disable the watchdog entirely.
    #[must_use]
    pub fn new(max_tick_time_ms: i64) -> Self {
        let max_tick_time_nanos = if max_tick_time_ms <= 0 {
            0
        } else {
            u64::try_from(max_tick_time_ms).unwrap_or(u64::MAX) * 1_000_000
        };
        Self {
            base: Instant::now(),
            tick_start_nanos: AtomicU64::new(0),
            in_tick: AtomicBool::new(false),
            max_tick_time_nanos,
        }
    }

    /// Record the instant a tick begins (monotonic nanos since the shared base).
    ///
    /// Stores the elapsed nanos, then sets `in_tick` to true with Release
    /// ordering so the watchdog sees both values atomically.
    pub fn begin_tick(&self) {
        let now = self.base.elapsed().as_nanos() as u64;
        self.tick_start_nanos.store(now, Ordering::Release);
        self.in_tick.store(true, Ordering::Release);
    }

    /// Mark the tick as finished.
    ///
    /// Sets `in_tick` to false with Release ordering.
    pub fn end_tick(&self) {
        self.in_tick.store(false, Ordering::Release);
    }

    /// Read the last recorded tick start time (nanos since base).
    pub fn tick_start_nanos(&self) -> u64 {
        self.tick_start_nanos.load(Ordering::Acquire)
    }

    /// Whether the server is currently inside a tick.
    pub fn in_tick(&self) -> bool {
        self.in_tick.load(Ordering::Acquire)
    }

    /// Current elapsed nanos since the shared base.
    pub fn now_nanos(&self) -> u64 {
        self.base.elapsed().as_nanos() as u64
    }

    /// Maximum allowed tick time in nanoseconds.
    pub const fn max_tick_time_nanos(&self) -> u64 {
        self.max_tick_time_nanos
    }
}

/// RAII guard that calls [`WatchdogState::end_tick`] on drop.
///
/// Ensures `in_tick` is cleared even if the tick body panics or returns early.
pub struct TickGuard<'a>(&'a WatchdogState);

impl<'a> TickGuard<'a> {
    pub const fn new(state: &'a WatchdogState) -> Self {
        Self(state)
    }
}

impl Drop for TickGuard<'_> {
    fn drop(&mut self) {
        self.0.end_tick();
    }
}

/// Decide whether the server should be considered stalled.
///
/// Returns `Some(elapsed_seconds)` when the server is inside a tick and the
/// elapsed time exceeds `max_tick_time` nanoseconds, mirroring vanilla's
/// `deltaNanos > maxTickTimeNanos` check.  Returns `None` otherwise.
///
/// Pure function, no side effects.
#[must_use]
pub fn stalled(
    in_tick: bool,
    tick_start_nanos: u64,
    now_nanos: u64,
    max_tick_time_nanos: u64,
) -> Option<f64> {
    if max_tick_time_nanos == 0 {
        return None;
    }
    if !in_tick {
        return None;
    }
    let elapsed_nanos = now_nanos.wrapping_sub(tick_start_nanos);
    if elapsed_nanos > max_tick_time_nanos {
        return Some(elapsed_nanos as f64 / 1_000_000_000.0);
    }
    None
}

/// Spawn the watchdog thread.
///
/// The watchdog loops while the server is running, sleeping until
/// `tick_start + max_tick_time` each iteration.  When a stall is detected it
/// logs the vanilla messages, dumps thread names, and calls `process::exit(1)`.
///
/// `milliseconds_per_tick` is a closure that returns the current
/// milliseconds-per-tick value, so that changes to the tick rate are reflected
/// in the stall message.
///
/// `stop_check` is a closure that returns `true` when the server is shutting down.
///
/// Returns `Err` if the thread could not be spawned (extremely rare).
pub fn spawn(
    max_tick_time_ms: i64,
    milliseconds_per_tick: Arc<dyn Fn() -> f64 + Send + Sync>,
    state: Arc<WatchdogState>,
    stop_check: Arc<dyn Fn() -> bool + Send + Sync>,
) -> std::io::Result<()> {
    if max_tick_time_ms <= 0 {
        return Ok(());
    }

    std::thread::Builder::new()
        .name("Server Watchdog".into())
        .spawn(move || {
            run(
                max_tick_time_ms,
                &milliseconds_per_tick,
                &state,
                &stop_check,
            );
        })
        .map(|_| ())
}

fn run(
    _max_tick_time_ms: i64,
    milliseconds_per_tick: &Arc<dyn Fn() -> f64 + Send + Sync>,
    state: &Arc<WatchdogState>,
    stop_check: &Arc<dyn Fn() -> bool + Send + Sync>,
) {
    let max_tick_time_nanos = state.max_tick_time_nanos();
    loop {
        if stop_check() {
            break;
        }

        let in_tick = state.in_tick();
        let tick_start_nanos = state.tick_start_nanos();
        let now_nanos = state.now_nanos();

        if let Some(elapsed_seconds) =
            stalled(in_tick, tick_start_nanos, now_nanos, max_tick_time_nanos)
        {
            // Stalled — log vanilla messages and exit.
            error!(
                "A single server tick took {:.2} seconds (should be max {})",
                elapsed_seconds,
                milliseconds_per_tick() / 1000.0
            );
            error!("Considering it to be crashed, server will forcibly shutdown.");
            thread_dump();
            std::process::exit(1);
        }

        // Sleep: when in a tick, sleep until tick_start + max_tick_time (at least 1ms).
        // When not in a tick, sleep a bounded amount (max_tick_time, capped at 1000ms).
        let sleep_ms = if in_tick {
            let deadline = tick_start_nanos.saturating_add(max_tick_time_nanos);
            let remaining_ns = deadline.saturating_sub(now_nanos);
            let remaining_ms = remaining_ns / 1_000_000;
            remaining_ms.max(1)
        } else {
            let max_sleep_ms = max_tick_time_nanos / 1_000_000;
            max_sleep_ms.min(1000)
        };

        std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
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
    use std::sync::Arc;

    const EPSILON: f64 = 1e-6;

    fn assert_f64_eq(actual: f64, expected: f64, msg: &str) {
        assert!(
            (actual - expected).abs() < EPSILON,
            "{msg}: expected {expected}, got {actual}"
        );
    }

    #[test]
    fn stalled_when_in_tick_and_exceeds_max() {
        // in_tick = true, elapsed (10_001_000_000 ns) > max (10_000_000_000 ns)
        let result = stalled(true, 0, 10_001_000_000, 10_000_000_000);
        assert!(result.is_some());
        assert_f64_eq(result.unwrap(), 10.001, "elapsed seconds");
    }

    #[test]
    fn not_stalled_at_exact_max() {
        // elapsed == max → should NOT fire (vanilla uses `>`, not `>=`)
        let result = stalled(true, 0, 10_000_000_000, 10_000_000_000);
        assert!(result.is_none());
    }

    #[test]
    fn not_stalled_below_max() {
        let result = stalled(true, 0, 5_000_000_000, 10_000_000_000);
        assert!(result.is_none());
    }

    #[test]
    fn not_stalled_between_ticks() {
        // in_tick = false → must not fire even if elapsed is huge
        let result = stalled(false, 0, 999_999_000_000_000, 10_000_000_000);
        assert!(result.is_none());
    }

    #[test]
    fn not_stalled_when_disabled() {
        let result = stalled(true, 0, 999_999_000_000_000, 0);
        assert!(result.is_none());
    }

    #[test]
    fn stalled_returns_elapsed_seconds() {
        let result = stalled(true, 0, 15_050_000_000, 10_000_000_000);
        assert_f64_eq(result.unwrap(), 15.05, "elapsed seconds");
    }

    /// Test begin_tick / end_tick flow with a real WatchdogState.
    #[test]
    fn begin_tick_and_end_tick_flow() {
        let state = WatchdogState::new(50);

        // Before begin_tick: not in tick
        assert!(!state.in_tick());
        assert_eq!(state.tick_start_nanos(), 0);

        // begin_tick: sets in_tick and records start time
        state.begin_tick();
        assert!(state.in_tick());
        let start = state.tick_start_nanos();
        assert!(start > 0);

        // After end_tick: not in tick
        state.end_tick();
        assert!(!state.in_tick());

        // Decision after end_tick should not be stalled
        let now = state.now_nanos();
        let result = stalled(state.in_tick(), start, now, state.max_tick_time_nanos());
        // elapsed is tiny, so not stalled
        assert!(result.is_none());
    }

    /// Test that the decision function works with controllable time values.
    #[test]
    fn controllable_time_stalled_while_in_tick_past_max() {
        let state = WatchdogState::new(50); // 50 ms = 50_000_000 ns

        state.begin_tick();
        let start = state.tick_start_nanos();

        // Simulate a time far past the max
        let now_nanos = start.saturating_add(60_000_000_000); // 60 seconds later

        let result = stalled(
            state.in_tick(),
            start,
            now_nanos,
            state.max_tick_time_nanos(),
        );
        assert!(result.is_some());
        assert_f64_eq(result.unwrap(), 60.0, "elapsed seconds past max");

        state.end_tick();

        // After end_tick: not stalled even with the same time
        let result = stalled(
            state.in_tick(),
            start,
            now_nanos,
            state.max_tick_time_nanos(),
        );
        assert!(result.is_none());
    }

    /// Test that the RAII guard clears in_tick on drop.
    #[test]
    fn raii_guard_clears_in_tick() {
        let state = Arc::new(WatchdogState::new(50));

        state.begin_tick();
        assert!(state.in_tick());

        {
            let _guard = TickGuard::new(&state);
            assert!(state.in_tick());
        }
        // Guard dropped → in_tick should be false
        assert!(!state.in_tick());
    }

    /// Test WatchdogState with custom max tick time.
    #[test]
    fn custom_max_tick_time() {
        let state = WatchdogState::new(100); // 100 ms
        assert_eq!(state.max_tick_time_nanos(), 100_000_000);

        let state2 = WatchdogState::new(-1);
        // Negative value should result in 0 (disabled)
        assert_eq!(state2.max_tick_time_nanos(), 0);
    }

    /// Test that max_tick_time is respected in the stalled decision.
    #[test]
    fn max_tick_time_respected_in_stalled() {
        // State with 60000 ms max: a tick that started 100 ms ago is NOT stalled.
        let state = WatchdogState::new(60_000); // 60000 ms = 60_000_000_000 ns
        assert_eq!(state.max_tick_time_nanos(), 60_000_000_000);
        let result = stalled(true, 0, 100_000_000, state.max_tick_time_nanos());
        assert!(
            result.is_none(),
            "100 ms elapsed should not stall with 60000 ms limit"
        );

        // State with 1000 ms max: a tick that started 2000 ms ago IS stalled.
        let state2 = WatchdogState::new(1_000); // 1000 ms = 1_000_000_000 ns
        assert_eq!(state2.max_tick_time_nanos(), 1_000_000_000);
        let result2 = stalled(true, 0, 2_000_000_000, state2.max_tick_time_nanos());
        assert!(
            result2.is_some(),
            "2000 ms elapsed should stall with 1000 ms limit"
        );
    }

    /// Test stalled function with max_tick_time_nanos = 0 (disabled).
    #[test]
    fn stalled_disabled_max() {
        let result = stalled(true, 0, u64::MAX, 0);
        assert!(result.is_none());
    }

    /// Test wrapping subtraction edge case.
    #[test]
    fn stalled_wrapping_sub() {
        // tick_start is near u64::MAX, now wraps to small value
        let tick_start: u64 = u64::MAX - 100;
        let now: u64 = 50;
        // wrapping_sub gives 150, which is small
        // Use a very small max to trigger the stall
        let result = stalled(true, tick_start, now, 100);
        // elapsed (150) > max (100) → stalled
        assert!(result.is_some());
    }
}
