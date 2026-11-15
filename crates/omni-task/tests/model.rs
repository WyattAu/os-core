//! REQ-OS-008 model test: the scheduler's contract stated as fairness
//! and sleep-spacing properties (fixed slots + a cursor is the
//! kernel-honest design; a deque oracle would encode different
//! rotation semantics).
//!
//! Properties:
//! 1. **Exact rotation**: with `k` live Continue-tasks and no churn,
//!    one window of `N` ticks dispatches every live task `N / k` times
//!    (±1 when `k` does not divide `N`).
//! 2. **Sleep spacing**: a task returning `Sleep(n)` is dispatched
//!    with consecutive dispatches at least `n` ticks apart, and its
//!    co-scheduled runner is never starved.
//! 3. **No starvation under churn**: through random spawns, `tick`
//!    never errors, and every still-live task is dispatched at least
//!    once in any `N`-tick window.
//!
//! `COUNTERS`/`SLEEP_QUANTUM` are THREAD-LOCAL: the three `#[test]` fns
//! run in parallel threads and share nothing — shared statics garbled
//! property 2's counts mid-run when property 1/3 reset the same cells.

use omni_task::{Scheduler, TaskCtx, TaskError, TaskFlow};
use proptest::prelude::*;

const N: usize = 4;

thread_local! {
    static COUNTERS: [std::sync::atomic::AtomicUsize; N] =
        const { [const { std::sync::atomic::AtomicUsize::new(0) }; N] };
    static SLEEP_QUANTUM: std::sync::atomic::AtomicUsize =
        const { std::sync::atomic::AtomicUsize::new(1) };
}

fn bump(counter: usize) {
    COUNTERS.with(|c| {
        if let Some(slot) = c.get(counter) {
            slot.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    });
}

fn reset_counters() {
    COUNTERS.with(|c| {
        for slot in c {
            slot.store(0, std::sync::atomic::Ordering::Relaxed);
        }
    });
}

fn counter(counter: usize) -> usize {
    COUNTERS.with(|c| {
        c.get(counter)
            .map_or(0, |a| a.load(std::sync::atomic::Ordering::Relaxed))
    })
}

fn set_quantum(q: usize) {
    SLEEP_QUANTUM.with(|q_cell| q_cell.store(q, std::sync::atomic::Ordering::Relaxed));
}

fn quantum() -> usize {
    SLEEP_QUANTUM.with(|q| q.load(std::sync::atomic::Ordering::Relaxed))
}

fn sleepy(ctx: &mut TaskCtx) -> TaskFlow {
    let _ = ctx;
    bump(0);
    TaskFlow::Sleep(quantum() as u64)
}

fn runner(_ctx: &mut TaskCtx) -> TaskFlow {
    bump(1);
    TaskFlow::Continue
}

fn never0(_: &mut TaskCtx) -> TaskFlow {
    bump(0);
    TaskFlow::Continue
}
fn never1(_: &mut TaskCtx) -> TaskFlow {
    bump(1);
    TaskFlow::Continue
}
fn never2(_: &mut TaskCtx) -> TaskFlow {
    bump(2);
    TaskFlow::Continue
}
fn never3(_: &mut TaskCtx) -> TaskFlow {
    bump(3);
    TaskFlow::Continue
}

const NEVER_FNS: [fn(&mut TaskCtx) -> TaskFlow; N] = [never0, never1, never2, never3];
const NAMES: [&str; N] = ["t0", "t1", "t2", "t3"];

/// The abort-on-impossible pattern: `unwrap`/`expect`/`panic` are
/// lint-denied in this crate's targets, and an aborted test process
/// fails loudly all the same.
macro_rules! must {
    ($e:expr, $msg:literal) => {
        match $e {
            Some(v) => v,
            None => std::process::abort(),
        }
    };
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 1: exact rotation with Continue-only tasks.
    #[test]
    fn exact_rotation(k in 1usize..=N) {
        reset_counters();
        let mut sched: Scheduler<N> = Scheduler::new();
        for ((slot, name), f) in NAMES.iter().enumerate().zip(NEVER_FNS.iter()).take(k) {
            let Ok(()) = sched.spawn(name, *f) else {
                std::process::abort();
            };
            let _ = slot;
        }
        let mut counts = [0usize; N];
        for _ in 0..N {
            let got = must!(sched.tick(), "a live task within the window");
            let Some(idx) = NAMES.iter().position(|n| *n == got) else {
                std::process::abort();
            };
            if let Some(c) = counts.get_mut(idx) {
                *c += 1;
            }
        }
        let lo = N / k;
        let hi = N.div_ceil(k);
        for ((_slot, &name), &c) in NAMES.iter().enumerate().zip(counts.iter()).take(k) {
            prop_assert!(c >= lo && c <= hi,
                "task {} ran {} times in {} ticks with {} live tasks", name, c, N, k);
        }
        for (empty_slot, &c) in counts.iter().enumerate().skip(k) {
            prop_assert_eq!(c, 0, "empty slot {} never dispatched", empty_slot);
        }
        prop_assert_eq!(sched.dispatches(), N as u64, "one dispatch per tick");
        prop_assert_eq!(sched.live(), k, "no churn");
    }

    /// Property 2: sleep spacing + fairness under sleep. The sleepy
    /// task always returns `Sleep(quantum)`: consecutive sleepy
    /// dispatches are at least `quantum` ticks apart (exactly quantum
    /// when the quantum exceeds the rotation period), the sleepy task
    /// runs ceil(W/quantum) times, and the runner takes every tick the
    /// sleeper skips.
    #[test]
    fn sleep_spacing_at_least_sleep(quantum in 1usize..=3usize) {
        reset_counters();
        set_quantum(quantum);
        let mut sched: Scheduler<N> = Scheduler::new();
        assert!(sched.spawn("sleepy", sleepy).is_ok(), "capacity holds");
        assert!(sched.spawn("runner", runner).is_ok(), "capacity holds");
        // Under the cursor rotation the spacing is max(quantum, N):
        // the quantum only bites when it exceeds the rotation period.
        // Ticks 1, 1+spacing, ... up through the last window tick are
        // ceil(W / spacing) dispatches.
        let window = 12 * (quantum + N);
        let mut sleepy_ticks: Vec<usize> = Vec::new();
        for t in 1..=window {
            if sched.tick() == Some("sleepy") {
                sleepy_ticks.push(t);
            }
        }
        prop_assert!(sleepy_ticks.len() >= 2, "sleepy dispatched repeatedly");
        for pair in sleepy_ticks.windows(2) {
            if let [a, b] = pair {
                let gap = b - a;
                prop_assert!(gap >= quantum,
                    "sleepy spacing {} below the Sleep({}) quantum", gap, quantum);
            }
        }
        prop_assert_eq!(counter(1), window - sleepy_ticks.len(),
            "the runner takes every skipped tick");
    }

    /// Property 3: no starvation through churn. Random spawns; in the
    /// trailing window every live task is dispatched at least once.
    #[test]
    fn no_starvation_under_churn(ops in proptest::collection::vec(0usize..N, 0..40)) {
        reset_counters();
        let mut sched: Scheduler<N> = Scheduler::new();
        let mut next = 0usize;
        for &id in &ops {
            if next < N {
                let (Some(name), Some(f)) =
                    (NAMES.get(next), NEVER_FNS.get(id))
                else {
                    std::process::abort();
                };
                let Ok(()) = sched.spawn(name, *f) else {
                    std::process::abort();
                };
                next += 1;
            } else {
                prop_assert_eq!(sched.spawn("x", never0),
                    Err(TaskError::Full), "full table rejects");
            }
        }
        let live = sched.live();
        let mut seen = [0usize; N];
        for _ in 0..N {
            if let Some(name) = sched.tick() {
                let Some(idx) = NAMES.iter().position(|n| *n == name) else {
                    std::process::abort();
                };
                if let Some(c) = seen.get_mut(idx) {
                    *c += 1;
                }
            }
        }
        for ((slot, &c), name) in seen.iter().enumerate().zip(NAMES.iter()) {
            if slot < live {
                prop_assert!(c >= 1, "live task {} dispatched in the window", name);
            } else {
                prop_assert_eq!(c, 0, "empty slot {} never dispatched", slot);
            }
        }
    }
}
