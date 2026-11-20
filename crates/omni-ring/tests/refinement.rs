//! Phase-3 refinement oracle (REQ-OS-001, ADR-0011): the ring theorems
//! in `proofs/Proofs.lean` — `ring_index_bounded`, `ring_mask_bounded`,
//! `ring_mask_injective`, `ring_window_injective`,
//! `ring_inflight_distinct` — are executed here against the real
//! `SpscRing` / `SlotTable`. The correspondence to the proofs is no
//! longer structural prose: a regression in the cursor/mask/window
//! arithmetic breaks these tests, and with them `make ci`.
//!
//! Each test names the theorem it executes.

use std::collections::VecDeque;

use omni_ring::slot::SlotTable;
use omni_ring::spsc::SpscRing;
use proptest::prelude::*;

/// The ring under test: `CAP = 2 ^ 3`, the `k = 3` of the theorems.
const CAP: usize = 8;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Executes `spsc_inflight_distinct` + the window injectivity
    /// engine (`proofs/Proofs.lean`, theorems of record): an arbitrary
    /// interleaving of pushes and
    /// pops must refine the FIFO model exactly. Distinct in-flight
    /// descriptors (both inside the `len() <= CAP` window) never alias
    /// a slot, so pop order equals push order across any number of
    /// window slides; `len()` tracks `head - tail` at every step.
    #[test]
    fn spsc_refines_fifo_across_wraparound(
        ops in proptest::collection::vec(any::<u8>(), 0..=64),
    ) {
        let ring: SpscRing<CAP> = SpscRing::new();
        let mut model: VecDeque<u64> = VecDeque::new();
        let mut next_desc: u64 = 1;
        for op in ops {
            match op % 3 {
                0 | 1 => {
                    let d = next_desc;
                    next_desc += 1;
                    if model.len() < CAP {
                        prop_assert_eq!(ring.try_push(d), None, "push accepted in window");
                        model.push_back(d);
                    } else {
                        prop_assert_eq!(
                            ring.try_push(d),
                            Some(d),
                            "full ring rejects with the descriptor unchanged"
                        );
                    }
                }
                _ => {
                    let expected = model.pop_front();
                    prop_assert_eq!(
                        ring.try_pop(),
                        expected,
                        "pop order equals push order (no slot aliasing)"
                    );
                }
            }
            prop_assert_eq!(ring.len(), model.len(), "len == head - tail (window accounting)");
        }
    }
}

/// Executes `spsc_slot_injective` (the def form of
/// `ring_mask_injective`) at the wrap seam: a full window
/// (in-flight == CAP) slid past the array boundary — push CAP, drain W,
/// push W fresh descriptors (their masked indices wrap), drain all —
/// comes back in exact push order. Equal masked indices for distinct
/// in-window descriptors would collapse slots and scramble this order.
#[test]
fn full_window_survives_wrap_seam() {
    let ring: SpscRing<CAP> = SpscRing::new();
    for d in 1..=CAP as u64 {
        assert_eq!(ring.try_push(d), None, "fresh ring absorbs CAP pushes");
    }
    assert!(ring.is_full());

    let w = 3usize;
    let drained: Vec<u64> = (0..w).filter_map(|_| ring.try_pop()).collect();
    assert_eq!(drained, vec![1, 2, 3], "first drain is FIFO");

    for j in 0..w as u64 {
        assert_eq!(
            ring.try_push(CAP as u64 + 1 + j),
            None,
            "freed window accepts the wrap-seam pushes"
        );
    }
    assert!(ring.is_full());

    for d in (w as u64 + 1)..=(CAP as u64 + w as u64) {
        assert_eq!(ring.try_pop(), Some(d), "post-wrap drain is exact FIFO");
    }
    assert_eq!(ring.try_pop(), None, "empty at the end");
}

/// Executes `spsc_inflight_distinct` at the payload layer: distinct
/// descriptors carry independent payloads and `take` is one-shot, so no
/// payload is delivered twice and nothing leaks between descriptors.
#[test]
fn slot_table_payloads_never_alias_or_double_deliver() {
    let mut table: SlotTable<u64, CAP> = SlotTable::new();
    for j in 0..CAP as u64 {
        assert_eq!(table.write(j, 1000 + j), Some(()), "write at j");
    }
    for j in 0..CAP as u64 {
        assert_eq!(table.take(j), Some(1000 + j), "payload j intact");
        assert_eq!(table.take(j), None, "take is one-shot at j");
    }
    let big = CAP as u64;
    assert_eq!(table.write(big, 7), None, "descriptor >= N rejected");
    assert_eq!(table.take(big), None, "descriptor >= N never delivers");
}
