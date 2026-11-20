//! Kani model-check harnesses (ADR-0014 rung C+ / ADR-0017): the
//! segment format's invariants proven over the actual Rust — the
//! hostile-count class (a forged header count past the buffer) is
//! rejected for ALL values, not sampled ones.

use crate::{SegmentBuf, Tick, HEADER_SIZE, TICK_SIZE};

fn any_tick() -> Tick {
    Tick {
        ts_ns: kani::any(),
        instrument: kani::any(),
        flags: kani::any(),
        bid_px: kani::any(),
        ask_px: kani::any(),
    }
}

#[kani::proof]
fn push_never_exceeds_capacity() {
    let mut backing = [0u8; HEADER_SIZE + 4 * TICK_SIZE];
    assert!(SegmentBuf::init(&mut backing).is_some(), "4-record inits");
    let Some(mut seg) = SegmentBuf::init(&mut backing) else {
        return;
    };
    let t = any_tick();
    for _ in 0..4 {
        assert!(seg.push(&t), "pushes up to capacity succeed");
    }
    assert!(!seg.push(&t), "the 5th push is rejected");
    assert_eq!(seg.count(), 4, "count never passes capacity");
}

#[kani::proof]
fn get_within_count_always_decodes() {
    let mut backing = [0u8; HEADER_SIZE + 3 * TICK_SIZE];
    assert!(SegmentBuf::init(&mut backing).is_some(), "3-record inits");
    let Some(mut seg) = SegmentBuf::init(&mut backing) else {
        return;
    };
    let t = any_tick();
    for _ in 0..3 {
        assert!(seg.push(&t), "pushes succeed");
    }
    let i: u32 = kani::any();
    kani::assume(i < 3);
    assert!(seg.get(i).is_some(), "get below count always decodes");
}

#[kani::proof]
fn open_rejects_count_past_buffer() {
    let mut backing = [0u8; HEADER_SIZE + 2 * TICK_SIZE];
    assert!(SegmentBuf::init(&mut backing).is_some(), "2-record inits");
    let Some(mut seg) = SegmentBuf::init(&mut backing) else {
        return;
    };
    assert!(seg.push(&any_tick()));
    drop(seg);
    // Forge the count header past the buffer's capacity: `open` must
    // reject EVERY hostile value (the hostile-count class).
    let forged: u32 = kani::any();
    kani::assume(forged > 2);
    if let Some(slot) = backing.get_mut(12..16) {
        slot.copy_from_slice(&forged.to_le_bytes());
    }
    assert!(
        SegmentBuf::open(&mut backing).is_none(),
        "forged count {forged} past capacity is rejected"
    );
}

#[kani::proof]
fn decode_of_encoded_is_identity() {
    let t = any_tick();
    let mut wire = [0u8; TICK_SIZE];
    t.encode(&mut wire);
    assert_eq!(Tick::decode(&wire), Some(t), "wire round trip is identity");
}
