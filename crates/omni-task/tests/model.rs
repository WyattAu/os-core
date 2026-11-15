//! REQ-OS-008 model test: the scheduler's contract stated as fairness
//! properties rather than trace-matching (fixed slots + a cursor is
//! the kernel-honest design; a deque oracle would encode a different
//! rotation semantics).
//!
//! Properties:
//! 1. **Exact rotation**: with `k` live tasks and no churn, one window
//!    of `N` ticks dispatches every live task `N / k` times (±1 when
//!    `k` does not divide `N`).
//! 2. **No starvation under churn**: through random spawns, `tick`
//!    never errors, and every still-live task is dispatched at least
//!    once in any `N`-tick window.

use omni_task::{Scheduler, TaskError};
use proptest::prelude::*;

const N: usize = 4;

static COUNTERS: [std::sync::atomic::AtomicUsize; N] = [
    const { std::sync::atomic::AtomicUsize::new(0) },
    const { std::sync::atomic::AtomicUsize::new(0) },
    const { std::sync::atomic::AtomicUsize::new(0) },
    const { std::sync::atomic::AtomicUsize::new(0) },
];

fn never0() -> bool {
    let _ = COUNTERS[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    false
}
fn never1() -> bool {
    let _ = COUNTERS[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    false
}
fn never2() -> bool {
    let _ = COUNTERS[2].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    false
}
fn never3() -> bool {
    let _ = COUNTERS[3].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    false
}

const NEVER_FNS: [fn() -> bool; N] = [never0, never1, never2, never3];
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

    /// Property 1: exact rotation. `k` live never-done tasks, one
    /// window of N ticks → every live task dispatched N/k times
    /// (±1 when k does not divide N), the rest of the table untouched.
    #[test]
    fn exact_rotation(k in 1usize..=N) {
        for c in &COUNTERS { c.store(0, std::sync::atomic::Ordering::Relaxed); }
        let mut sched: Scheduler<N> = Scheduler::new();
        for ((i, name), f) in
            NAMES.iter().enumerate().zip(NEVER_FNS.iter()).take(k)
        {
            let Ok(()) = sched.spawn(name, *f) else {
                std::process::abort();
            };
            let _ = i;
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
        for (i, (&name, &c)) in NAMES.iter().zip(counts.iter()).enumerate() {
            if i < k {
                prop_assert!(c >= lo && c <= hi,
                    "task {} ran {} times in {} ticks with {} live tasks", name, c, N, k);
            } else {
                prop_assert_eq!(c, 0, "empty slot {} never dispatched", i);
            }
        }
        prop_assert_eq!(sched.dispatches(), N as u64, "one dispatch per tick");
        prop_assert_eq!(sched.live(), k, "no churn");
    }

    /// Property 2: no starvation through churn. Random spawns; in the
    /// trailing window every live task is dispatched at least once.
    #[test]
    fn no_starvation_under_churn(ops in proptest::collection::vec(0usize..N, 0..40)) {
        for c in &COUNTERS { c.store(0, std::sync::atomic::Ordering::Relaxed); }
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
                prop_assert_eq!(sched.spawn("x", NEVER_FNS[0]),
                    Err(TaskError::Full), "full table rejects");
            }
        }
        // Trailing window: every live task appears at least once.
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
        for ((i, &c), name) in seen.iter().enumerate().zip(NAMES.iter()) {
            if i < live {
                prop_assert!(c >= 1, "live task {} dispatched in the window", name);
            } else {
                prop_assert_eq!(c, 0, "empty slot {} never dispatched", i);
            }
        }
    }
}
