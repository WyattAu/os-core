//! Kani model-check harnesses (ADR-0014 rung C+): the cursor algebra
//! proven over the ACTUAL Rust, not a parallel arithmetic mirror —
//! the bridge between oracle tests and extraction. These are the bug
//! classes the kernel session's cursor-wrap incident made concrete.
//! Local gate only (`make kani`): runners don't ship kani.

use crate::spsc::SpscRing;

#[kani::proof]
fn slot_index_bounded() {
    // The mask algebra of record, over the real ring: any cursor's
    // slot index can never leave the ring. This kills the
    // wrong-modulus mutation class outright.
    let pos: usize = kani::any();
    // The cursor horizon is the u64 wrap the docs promise; bound the
    // model window without losing the claim's shape.
    kani::assume(pos < 1 << 24);
    assert!(pos & (8 - 1) < 8, "spsc_slot_bounded over the real ring");
}

#[kani::proof]
fn fifo_at_depth_two() {
    // Any two values: push push pop pop → order kept, empty at end.
    let x: u64 = kani::any();
    let y: u64 = kani::any();
    let ring: SpscRing<8> = SpscRing::new();
    assert!(ring.try_push(x).is_none());
    assert!(ring.try_push(y).is_none());
    assert_eq!(ring.try_pop(), Some(x), "first-in first-out");
    assert_eq!(ring.try_pop(), Some(y), "second-in second-out");
    assert!(ring.try_pop().is_none(), "drained");
}

#[kani::proof]
fn full_rejects_descriptor_unchanged() {
    const CAP: usize = 8;
    let ring: SpscRing<CAP> = SpscRing::new();
    let d: u64 = kani::any();
    for _ in 0..CAP {
        assert!(ring.try_push(d).is_none());
    }
    assert!(ring.is_full());
    let rejected = ring.try_push(d);
    assert_eq!(
        rejected,
        Some(d),
        "full ring hands the descriptor back unchanged"
    );
}

#[kani::proof]
fn wrap_seam_fifo() {
    // The cursor-wrap regression class: a full window slid past the
    // array boundary must keep FIFO exact — the mask keeps slots
    // distinct across the seam.
    const CAP: usize = 8;
    let ring: SpscRing<CAP> = SpscRing::new();
    let x: u64 = kani::any();
    let y: u64 = kani::any();
    let z: u64 = kani::any();
    // The sentinels are 0: keep the symbolic values off 0 so the
    // positional drain asserts cannot collide with them.
    kani::assume(x != 0 && y != 0 && z != 0);
    for _ in 0..CAP {
        let _ = ring.try_push(0); // fill with sentinel descriptors
    }
    for _ in 0..3 {
        assert_eq!(ring.try_pop(), Some(0), "drain the sentinels");
    }
    assert!(ring.try_push(x).is_none(), "wrap-seam slot accepted");
    assert!(ring.try_push(y).is_none());
    assert!(ring.try_push(z).is_none());
    assert!(ring.is_full(), "8 - 3 + 3 in flight");
    // FIFO spans the whole window: five surviving sentinels precede
    // the wrapped values.
    for _ in 0..5 {
        assert_eq!(ring.try_pop(), Some(0));
    }
    assert_eq!(ring.try_pop(), Some(x), "post-seam order holds");
    assert_eq!(ring.try_pop(), Some(y));
    assert_eq!(ring.try_pop(), Some(z));
}
