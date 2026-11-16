//! # omni-task — deterministic cooperative task slots (REQ-OS-008)
//!
//! Round-robin dispatch over fixed task slots, driven by one tick call
//! (the PIT handler calls [`Scheduler::tick`] once per tick). Design:
//!
//! - `no_std`, allocation-free, panic-free (every failure is a
//!   `Result`/`Option`) — same posture as `omni-heap`.
//! - **Deterministic**: one tick dispatches exactly one awake task, the
//!   next awake task after the cursor — a given spawn/tick sequence
//!   dispatches identically on every boot.
//! - **Cooperative sleep is a task state**: a task returns
//!   [`TaskFlow::Sleep(n)`] and its slot skips dispatch until `n` ticks
//!   have passed (`TaskFlow::Done` retires the slot).
//! - Tasks run in tick (interrupt) context: they must not block.

#![no_std]

/// Every fallible operation's failure mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskError {
    /// `spawn` with no free slot.
    Full,
}

/// Per-dispatch context handed to a task fn.
#[derive(Debug)]
pub struct TaskCtx {
    /// Ticks observed so far (the scheduler's monotonic tick count).
    pub ticks: u64,
}

/// What a task wants after its dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskFlow {
    /// The task is finished: its slot retires.
    Done,
    /// Keep the slot live; dispatch again on the next rotation.
    Continue,
    /// Keep the slot live; skip dispatch until `n` more ticks have
    /// passed (cooperative sleep, tick-quantized).
    Sleep(u64),
}

/// One registered task: a `run` fn plus its lifecycle state.
#[derive(Clone, Copy)]
pub struct TaskSlot {
    /// Runs once per dispatch (awake, cursor on this slot).
    pub run: fn(&mut TaskCtx) -> TaskFlow,
    /// Stable identifier surfaced by `tick` for dispatch tracing.
    pub name: &'static str,
    /// Wake tick: the slot sleeps while `wake_at > ticks`.
    wake_at: u64,
}

/// Round-robin dispatcher over `N` fixed slots.
pub struct Scheduler<const N: usize> {
    slots: [Option<TaskSlot>; N],
    /// Round-robin cursor: index of the next slot to examine.
    cursor: usize,
    /// Total ticks observed.
    ticks: u64,
    /// Total dispatches performed.
    dispatches: u64,
}

impl<const N: usize> Scheduler<N> {
    /// Creates an empty scheduler.
    #[must_use]
    #[allow(clippy::new_without_default)] // the kernel uses `new` directly
    pub const fn new() -> Self {
        Self {
            slots: [const { None }; N],
            cursor: 0,
            ticks: 0,
            dispatches: 0,
        }
    }

    /// Registers a task in the first free slot.
    ///
    /// # Errors
    ///
    /// [`TaskError::Full`] when every slot is occupied.
    pub fn spawn(
        &mut self,
        name: &'static str,
        run: fn(&mut TaskCtx) -> TaskFlow,
    ) -> Result<(), TaskError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|s| s.is_none())
            .ok_or(TaskError::Full)?;
        *slot = Some(TaskSlot {
            run,
            name,
            wake_at: 0,
        });
        Ok(())
    }

    /// Number of live (not retired) tasks — sleeping tasks are live.
    #[must_use]
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Ticks observed so far.
    #[must_use]
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Dispatches performed so far.
    #[must_use]
    pub const fn dispatches(&self) -> u64 {
        self.dispatches
    }

    /// One tick: dispatch the next AWAKE task after the cursor
    /// (round-robin, skipping done and sleeping slots). Returns the
    /// dispatched task's name, or `None` when nothing dispatched
    /// (empty table or every task asleep).
    ///
    /// The cursor never skips backwards and the scan is bounded by
    /// `N` — dispatch is wait-free with try semantics, mirroring the
    /// rings' posture.
    pub fn tick(&mut self) -> Option<&'static str> {
        self.ticks += 1;
        let mut ctx = TaskCtx { ticks: self.ticks };
        for step in 0..N {
            let idx = (self.cursor + step) % N;
            let Some(task) = self.slots.get_mut(idx) else {
                continue;
            };
            let Some(task_slot) = task else {
                continue;
            };
            // Sleeping slot: skip without consuming a dispatch.
            if task_slot.wake_at > self.ticks {
                continue;
            }
            let flow = (task_slot.run)(&mut ctx);
            self.dispatches += 1;
            self.cursor = (idx + 1) % N;
            let name = task_slot.name;
            match flow {
                TaskFlow::Sleep(n) => {
                    task_slot.wake_at = self.ticks.saturating_add(n);
                }
                TaskFlow::Done => {
                    // Retire through the borrow: no second indexing.
                    *task = None;
                }
                TaskFlow::Continue => {}
            }
            return Some(name);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    // Per-test counters: tests run in PARALLEL threads — a shared
    // static would interleave (a parity-based retire here once flapped
    // on a macos CI runner for exactly that reason).
    static RR_COUNT: AtomicUsize = AtomicUsize::new(0);
    static RETIRE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static SLEEP_RR_COUNT: AtomicUsize = AtomicUsize::new(0);

    fn bump_rr(_: &mut TaskCtx) -> TaskFlow {
        RR_COUNT.fetch_add(1, Ordering::Relaxed);
        TaskFlow::Continue
    }

    fn bump_retire(_: &mut TaskCtx) -> TaskFlow {
        let n = RETIRE_COUNT.fetch_add(1, Ordering::Relaxed);
        if n + 1 >= 3 {
            TaskFlow::Done
        } else {
            TaskFlow::Continue
        }
    }

    #[test]
    fn round_robin_order_is_deterministic() {
        RR_COUNT.store(0, Ordering::Relaxed);
        let mut s: Scheduler<4> = Scheduler::new();
        assert!(s.spawn("a", bump_rr).is_ok());
        assert!(s.spawn("b", bump_rr).is_ok());
        assert!(s.spawn("c", bump_rr).is_ok());
        assert_eq!(s.tick(), Some("a"));
        assert_eq!(s.tick(), Some("b"));
        assert_eq!(s.tick(), Some("c"));
        assert_eq!(s.dispatches(), 3);
    }

    #[test]
    fn tick_with_no_tasks_is_none() {
        let mut s: Scheduler<4> = Scheduler::new();
        assert_eq!(s.tick(), None);
        assert_eq!(s.ticks(), 1, "the tick is still counted");
    }

    #[test]
    fn done_tasks_retire_and_free_their_slot() {
        RETIRE_COUNT.store(0, Ordering::Relaxed);
        let mut s: Scheduler<4> = Scheduler::new();
        assert!(s.spawn("x", bump_retire).is_ok());
        assert_eq!(s.tick(), Some("x"));
        assert_eq!(s.live(), 1);
        assert_eq!(s.tick(), Some("x"));
        assert_eq!(s.tick(), Some("x"), "third dispatch returns done");
        assert_eq!(s.live(), 0, "done task retired");
        assert_eq!(s.tick(), None);
        // The retired slot is reusable.
        assert!(s.spawn("y", bump_rr).is_ok());
        assert_eq!(s.tick(), Some("y"));
    }

    #[test]
    fn spawn_full_is_an_error() {
        let mut s: Scheduler<2> = Scheduler::new();
        assert!(s.spawn("a", bump_rr).is_ok());
        assert!(s.spawn("b", bump_rr).is_ok());
        assert_eq!(s.spawn("c", bump_rr), Err(TaskError::Full));
    }

    static SLEEPY_COUNT: AtomicUsize = AtomicUsize::new(0);

    fn sleepy_always(_: &mut TaskCtx) -> TaskFlow {
        SLEEPY_COUNT.fetch_add(1, Ordering::Relaxed);
        TaskFlow::Sleep(2)
    }

    fn bump_sleep_runner(_: &mut TaskCtx) -> TaskFlow {
        SLEEP_RR_COUNT.fetch_add(1, Ordering::Relaxed);
        TaskFlow::Continue
    }

    #[test]
    fn sleeping_slots_skip_without_starving_others() {
        RR_COUNT.store(0, Ordering::Relaxed);
        SLEEPY_COUNT.store(0, Ordering::Relaxed);
        let mut s: Scheduler<4> = Scheduler::new();
        // A task that sleeps 2 ticks after every dispatch.
        assert!(s.spawn("sleepy", sleepy_always).is_ok());
        assert!(s.spawn("runner", bump_sleep_runner).is_ok());
        // 8 ticks with Sleep(2): sleepy dispatches every 2 ticks
        // (1, 3, 5, 7 = 4 dispatches); the runner takes the other 4 —
        // exact counts, no starvation either way.
        for _ in 0..8 {
            s.tick();
        }
        assert_eq!(SLEEPY_COUNT.load(Ordering::Relaxed), 4, "sleepy spacing");
        assert_eq!(
            SLEEP_RR_COUNT.load(Ordering::Relaxed),
            4,
            "runner not starved"
        );
    }
}
