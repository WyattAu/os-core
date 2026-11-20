//! Descriptor-ring fan-out: publish pushes the tick into the payload
//! table and its sequence number into the ring; the consumer pops the
//! descriptor and takes the payload. This is `io_uring`'s model at
//! micro scale (omni-ring's docs), applied to the market-data path.

use omni_ring::slot::SlotTable;
use omni_ring::spsc::SpscRing;
use omni_tick::Tick;

/// A single-producer/single-consumer tick fan-out of `CAP` in-flight
/// ticks. Publish is the producer's doorbell; drain is the consumer's
/// reap — the same two-ring shape the `io_uring` seam (omni-io) runs at
/// kernel scale.
pub struct Fanout<const CAP: usize> {
    ring: SpscRing<CAP>,
    table: SlotTable<Tick, CAP>,
}

impl<const CAP: usize> Fanout<CAP> {
    /// Creates an empty fan-out.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ring: SpscRing::new(),
            table: SlotTable::new(),
        }
    }

    /// Publishes `tick` under descriptor `seq`. `false` when the
    /// slot table rejects the descriptor (seq beyond the table) or
    /// the ring is full — the caller drops or backpressures (never
    /// blocks: the pipeline is try-semantics end to end).
    ///
    /// # Panics
    ///
    /// Only if the full-ring unwind fails to restore the rejected
    /// tick — an internal invariant whose violation means the
    /// fan-out state is corrupt, not a caller error.
    pub fn publish(&mut self, seq: u64, tick: &Tick) -> bool {
        // Slot first, descriptor second: a full-ring rejection then
        // unwinds with the one-shot `take`, which restores exactly the
        // empty slot — a FIFO `try_pop` would roll back the OLDEST
        // descriptor, not the rejected one.
        if self.table.write(seq, *tick).is_none() {
            return false;
        }
        if self.ring.try_push(seq).is_some() {
            let unwound = self.table.take(seq);
            assert!(
                unwound.as_ref() == Some(tick),
                "unwind must restore exactly the rejected tick"
            );
            return false;
        }
        true
    }

    /// Drains one published tick, in publish order. `None` when the
    /// fan-out is empty.
    pub fn drain_one(&mut self) -> Option<Tick> {
        let seq = self.ring.try_pop()?;
        self.table.take(seq)
    }

    /// Ticks currently in flight.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// True when nothing is in flight.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

impl<const CAP: usize> Default for Fanout<CAP> {
    fn default() -> Self {
        Self::new()
    }
}
