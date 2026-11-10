//! Wait-free bounded SPSC ring over `u64` descriptors (REQ-OS-001).
//!
//! Single producer, single consumer; `try_push` / `try_pop` only, so every
//! operation completes in a bounded number of steps regardless of
//! contention (wait-free with try semantics).
//!
//! Protocol: cursors `head` (producer) and `tail` (consumer) advance
//! monotonically and never wrap for the practical lifetime of the process
//! (`u64`/platform-usize horizon; documented, not assumed silently). The
//! slot index is `cursor & (CAP - 1)` — machine-checked bounded in
//! `proofs/` as `ring_index_bounded`.
//!
//! Ordering: producer writes the slot with `Release` *before* advancing
//! `head` with `Release`; consumer reads `head` with `Acquire`, then the
//! slot, then advances `tail` with `Release`; producer reads `tail` with
//! `Acquire` before reusing a slot. That pair closes both edges of the
//! wraparound reuse race. Zero `SeqCst`; zero `unsafe`.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Pads a cursor to its own cache line so producer and consumer never
/// false-share (goals.md §161).
#[repr(align(64))]
struct Padded(AtomicUsize);

/// A bounded wait-free SPSC ring of `u64` descriptors.
///
/// `CAP` must be a positive power of two (checked at compile time).
pub struct SpscRing<const CAP: usize> {
    head: Padded,
    tail: Padded,
    slots: [AtomicU64; CAP],
}

impl<const CAP: usize> SpscRing<CAP> {
    /// Creates the ring. `CAP` is validated at compile time.
    #[must_use]
    pub fn new() -> Self {
        const {
            assert!(
                CAP > 0 && CAP.is_power_of_two(),
                "CAP must be a positive power of two"
            );
        }
        Self {
            head: Padded(AtomicUsize::new(0)),
            tail: Padded(AtomicUsize::new(0)),
            slots: core::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    /// The ring capacity (`CAP`).
    #[must_use]
    pub const fn capacity(&self) -> usize {
        CAP
    }

    /// Number of descriptors currently in flight.
    #[must_use]
    pub fn len(&self) -> usize {
        let head = self.head.0.load(Ordering::Acquire);
        let tail = self.tail.0.load(Ordering::Acquire);
        head - tail
    }

    /// True when no descriptor is in flight.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when the ring cannot accept another descriptor.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.len() == CAP
    }

    /// Enqueues `descriptor`, or returns it unchanged when the ring is full.
    ///
    /// Wait-free: a fixed number of atomic operations regardless of the
    /// consumer's progress.
    pub fn try_push(&self, descriptor: u64) -> Option<u64> {
        let pos = self.head.0.load(Ordering::Relaxed);
        // Acquire pairs with the consumer's `Release` tail advance: the
        // slot the consumer just drained is safe to overwrite only after
        // this load observes its retirement.
        let tail = self.tail.0.load(Ordering::Acquire);
        if pos - tail >= CAP {
            return Some(descriptor); // full
        }
        let Some(slot) = self.slots.get(pos & (CAP - 1)) else {
            // Unreachable by `ring_index_bounded` (proofs/): the mask keeps
            // indices below `CAP`. Fail closed by returning the value.
            return Some(descriptor);
        };
        slot.store(descriptor, Ordering::Release);
        self.head.0.store(pos + 1, Ordering::Release);
        None
    }

    /// Dequeues the oldest descriptor, or `None` when the ring is empty.
    ///
    /// Wait-free: a fixed number of atomic operations regardless of the
    /// producer's progress.
    #[must_use]
    pub fn try_pop(&self) -> Option<u64> {
        let tail = self.tail.0.load(Ordering::Relaxed);
        if self.head.0.load(Ordering::Acquire) == tail {
            return None; // empty
        }
        let Some(slot) = self.slots.get(tail & (CAP - 1)) else {
            // Unreachable by `ring_index_bounded` (proofs/). Fail closed.
            return None;
        };
        // Relaxed is sound here: the producer's slot write happens-before
        // this read via the producer's `Release` head advance and this
        // function's `Acquire` head load.
        let descriptor = slot.load(Ordering::Relaxed);
        self.tail.0.store(tail + 1, Ordering::Release);
        Some(descriptor)
    }
}

impl<const CAP: usize> Default for SpscRing<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> core::fmt::Debug for SpscRing<CAP> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SpscRing")
            .field("capacity", &CAP)
            .field("len", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::SpscRing;

    #[test]
    fn fifo_order_is_preserved() {
        let ring: SpscRing<8> = SpscRing::new();
        for v in 1..=8_u64 {
            assert_eq!(ring.try_push(v), None, "ring must absorb CAP pushes");
        }
        assert!(ring.is_full());
        assert_eq!(ring.try_push(9), Some(9), "full ring returns the value");
        for v in 1..=8_u64 {
            assert_eq!(ring.try_pop(), Some(v), "FIFO order must hold");
        }
        assert!(ring.is_empty());
        assert_eq!(ring.try_pop(), None);
    }

    #[test]
    fn wraparound_reuses_slots_in_order() {
        let ring: SpscRing<2> = SpscRing::new();
        for v in 1..=6_u64 {
            assert_eq!(ring.try_push(v), None);
            assert_eq!(ring.try_pop(), Some(v));
        }
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn overflow_return_is_fail_closed() {
        let ring: SpscRing<4> = SpscRing::new();
        assert_eq!(ring.try_pop(), None);
        assert_eq!(ring.capacity(), 4);
    }

    #[test]
    fn debug_default_and_fullness_are_usable() {
        let ring: SpscRing<4> = SpscRing::default();
        assert!(ring.is_empty());
        assert!(format!("{ring:?}").contains("capacity: 4"));
        for v in 0..4_u64 {
            assert_eq!(ring.try_push(v), None);
        }
        assert!(ring.is_full());
        assert!(!ring.is_empty());
        assert_eq!(ring.len(), 4);
        assert!(format!("{ring:?}").contains("len: 4"));
    }
}
