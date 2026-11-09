//! Lock-free bounded MPMC ring (REQ-OS-001).
//!
//! Delegates to [`crossbeam_queue::ArrayQueue`] — the estate precedent
//! (actor-kit mailboxes, ferro event-bus). Lock-free, not wait-free: under
//! heavy contention a `push`/`pop` can retry internally. The distinction
//! from [`crate::spsc::SpscRing`] is stated per OMNI-CORE 11f, not blurred.

use crossbeam_queue::ArrayQueue;

/// A bounded lock-free MPMC ring of `T`.
#[derive(Debug)]
pub struct MpmcRing<T> {
    queue: ArrayQueue<T>,
}

impl<T> MpmcRing<T> {
    /// Creates a ring holding at most `capacity` items.
    ///
    /// # Panics
    ///
    /// Panics when `capacity` is zero.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be positive");
        Self {
            queue: ArrayQueue::new(capacity),
        }
    }

    /// The ring capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.queue.capacity()
    }

    /// Number of items currently in flight.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// True when no item is in flight.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Enqueues `item`, or returns it unchanged when the ring is full.
    ///
    /// # Errors
    ///
    /// Returns `Err(item)` when the ring is full; the item is handed back
    /// and the queue is unchanged.
    pub fn push(&self, item: T) -> Result<(), T> {
        self.queue.push(item)
    }

    /// Dequeues the oldest item, or `None` when the ring is empty.
    #[must_use]
    pub fn pop(&self) -> Option<T> {
        self.queue.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::MpmcRing;

    #[test]
    fn fifo_under_interleaved_single_thread() {
        let ring: MpmcRing<u64> = MpmcRing::new(4);
        assert!(ring.push(1).is_ok());
        assert!(ring.push(2).is_ok());
        assert_eq!(ring.pop(), Some(1));
        assert!(ring.push(3).is_ok());
        assert_eq!(ring.pop(), Some(2));
        assert_eq!(ring.pop(), Some(3));
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn full_ring_returns_item() {
        let ring: MpmcRing<u64> = MpmcRing::new(1);
        assert!(ring.push(7).is_ok());
        assert_eq!(ring.push(8), Err(8));
        assert_eq!(ring.pop(), Some(7));
        assert!(ring.is_empty());
    }

    #[test]
    fn capacity_len_and_debug_delegate() {
        let ring: MpmcRing<u64> = MpmcRing::new(9);
        assert_eq!(ring.capacity(), 9);
        assert!(ring.is_empty());
        assert!(ring.push(1).is_ok());
        assert!(!ring.is_empty());
        assert_eq!(ring.len(), 1);
        assert!(format!("{ring:?}").contains("MpmcRing"));
    }
}
