//! Typed slot table for descriptor transport (REQ-OS-001 follow-up).
//!
//! The generic-payload follow-up for the SPSC ring resolves at the
//! architecture layer, not the atomics layer: rings carry `u64`
//! descriptors (the Lean-provable surface — `ring_index_bounded`), and
//! a [`SlotTable`] gives those descriptors typed storage. This is
//! `io_uring`'s model — the queue moves indices, the data lives in slots
//! — and it keeps the estate's `unsafe_code = deny` contract intact:
//! the hand-rolled ring stays zero-unsafe, and the table adds nothing
//! that could break that.
//!
//! Contract (mirrored by `tests/model.rs`):
//! - `write`/`read` are bounds-checked (`Option`), never panic.
//! - A descriptor `d` written before it is pushed yields the same `T`
//!   when read after it is popped (typed FIFO end-to-end).

/// A fixed-capacity table of typed slots addressed by ring descriptors.
///
/// `T: Copy` keeps the transport semantics identical to the ring's
/// descriptors (no drop obligations).
pub struct SlotTable<T, const N: usize> {
    slots: [Option<T>; N],
}

impl<T: Copy, const N: usize> SlotTable<T, N> {
    /// Creates an all-empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: [const { None }; N],
        }
    }

    /// The table capacity (`N`).
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Writes `value` at descriptor `d`; `None` when `d >= N`.
    pub fn write(&mut self, d: u64, value: T) -> Option<()> {
        // Guard the conversion first: `d >= N` can never truncate.
        // Lossless widening conversion; `i >= N` covers the bound.
        let i = usize::try_from(d).ok()?;
        if i >= N {
            return None;
        }
        let slot = self.slots.get_mut(i)?;
        *slot = Some(value);
        Some(())
    }

    /// Takes the value at descriptor `d`, leaving the slot empty;
    /// `None` when `d >= N` or the slot was never written (or was
    /// already taken).
    pub fn take(&mut self, d: u64) -> Option<T> {
        let i = usize::try_from(d).ok()?;
        if i >= N {
            return None;
        }
        let slot = self.slots.get_mut(i)?;
        slot.take()
    }
}

impl<T: Copy, const N: usize> Default for SlotTable<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_bounds() {
        let mut t: SlotTable<u64, 4> = SlotTable::new();
        assert_eq!(t.capacity(), 4);
        assert_eq!(t.write(2, 42), Some(()));
        assert_eq!(t.take(2), Some(42));
        assert_eq!(t.take(2), None, "take empties the slot");
        assert_eq!(t.write(4, 1), None, "descriptor >= N is rejected");
        assert_eq!(t.take(4), None);
    }
}
