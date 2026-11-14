//! # omni-task — deterministic cooperative task slots (REQ-OS-008)
//!
//! Round-robin dispatch over fixed task slots, driven by one tick call
//! (the PIT handler calls [`Scheduler::tick`] once per tick). Design:
//!
//! - `no_std`, allocation-free, panic-free (every failure is a
//!   `Result`/`Option`) — same posture as `omni-heap`.
//! - **Deterministic**: one tick dispatches exactly one live task, the
//!   next live task after the cursor — a given spawn/tick sequence
//!   dispatches identically on every boot.
//! - Tasks retire themselves: a task fn returning `true` marks the slot
//!   done; `tick` skips done slots until the table is empty.
//!
//! Tasks run in tick (interrupt) context: they must not block. That
//! contract is checked by review, not by code — the smoke's sleep is a
//! task-context user and deliberately stays outside this table.

#![no_std]

/// Every fallible operation's failure mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskError {
    /// `spawn` with no free slot.
    Full,
}

/// One registered task: a `run` fn plus its lifecycle flags.
#[derive(Clone, Copy)]
pub struct TaskSlot {
    /// Runs once per dispatch; returns `true` when the task is done
    /// and its slot may retire.
    pub run: fn() -> bool,
    /// Stable identifier surfaced by `tick` for dispatch tracing.
    pub name: &'static str,
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
    #[allow(clippy::new_without_default)] // the kernel uses `new` directly; a
                                          // Default impl would suggest runtime-created schedulers
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
    pub fn spawn(&mut self, name: &'static str, run: fn() -> bool) -> Result<(), TaskError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|s| s.is_none())
            .ok_or(TaskError::Full)?;
        *slot = Some(TaskSlot { run, name });
        Ok(())
    }

    /// Number of live (not done) tasks.
    #[must_use]
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Ticks observed so far.
    #[must_use]
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Dispatches performed so far.
    #[must_use]
    pub fn dispatches(&self) -> u64 {
        self.dispatches
    }

    /// One tick: dispatch the next live task after the cursor
    /// (round-robin, skipping done slots). Returns the dispatched
    /// task's name, or `None` when no live task exists.
    ///
    /// The cursor never skips backwards and the scan is bounded by
    /// `N` — dispatch is wait-free with try semantics, mirroring the
    /// rings' posture.
    pub fn tick(&mut self) -> Option<&'static str> {
        self.ticks += 1;
        for step in 0..N {
            let idx = (self.cursor + step) % N;
            let Some(task) = self.slots.get_mut(idx) else {
                continue;
            };
            let Some(task_slot) = task else {
                continue;
            };
            let done = (task_slot.run)();
            self.dispatches += 1;
            self.cursor = (idx + 1) % N;
            let name = task_slot.name;
            if done {
                // Retire through the borrow: no second indexing.
                *task = None;
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

    static RUN_COUNT: AtomicUsize = AtomicUsize::new(0);

    fn bump() -> bool {
        RUN_COUNT.fetch_add(1, Ordering::Relaxed);
        false // never done
    }

    fn bump_twice_then_done() -> bool {
        let n = RUN_COUNT.fetch_add(1, Ordering::Relaxed);
        n % 2 == 1
    }

    #[test]
    fn round_robin_order_is_deterministic() {
        RUN_COUNT.store(0, Ordering::Relaxed);
        let mut s: Scheduler<4> = Scheduler::new();
        assert!(s.spawn("a", bump).is_ok());
        assert!(s.spawn("b", bump).is_ok());
        assert!(s.spawn("c", bump).is_ok());
        // Three ticks dispatch a, b, c in spawn order.
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
        RUN_COUNT.store(0, Ordering::Relaxed);
        let mut s: Scheduler<4> = Scheduler::new();
        assert!(s.spawn("x", bump_twice_then_done).is_ok());
        assert_eq!(s.tick(), Some("x"));
        assert_eq!(s.live(), 1);
        assert_eq!(s.tick(), Some("x"));
        assert_eq!(s.live(), 0, "done task retired");
        assert_eq!(s.tick(), None);
        // The retired slot is reusable.
        assert!(s.spawn("y", bump).is_ok());
        assert_eq!(s.tick(), Some("y"));
    }

    #[test]
    fn spawn_full_is_an_error() {
        let mut s: Scheduler<2> = Scheduler::new();
        assert!(s.spawn("a", bump).is_ok());
        assert!(s.spawn("b", bump).is_ok());
        assert_eq!(s.spawn("c", bump), Err(TaskError::Full));
    }
}
