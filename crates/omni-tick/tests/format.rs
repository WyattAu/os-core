//! omni-tick format tests: tick wire roundtrip, segment append/replay,
//! header validation, and hostile-extent rejection (REQ-OS-010).

use omni_tick::{SegmentBuf, Tick, HEADER_SIZE, TICK_SIZE};

fn tick(i: u64) -> Tick {
    Tick {
        ts_ns: 1_700_000_000_000_000_000 + i * 100,
        instrument: 42,
        flags: u32::try_from(i).unwrap_or_default(),
        bid_px: 4_295_032_500 + i,
        ask_px: 4_295_033_000 + i,
    }
}

/// Fails unless `Some`, returning the value (house assert+abort style).
fn must_some<T>(v: Option<T>, what: &str) -> T {
    assert!(v.is_some(), "{what}");
    match v {
        Some(v) => v,
        None => std::process::abort(),
    }
}

#[test]
fn tick_wire_roundtrip() {
    let mut wire = [0u8; TICK_SIZE];
    let t = tick(7);
    t.encode(&mut wire);
    let back = Tick::decode(&wire);
    assert!(back.is_some(), "full-size wire decodes");
    assert_eq!(back.unwrap_or(tick(0)), t, "round trip exact");
}

#[test]
fn short_wire_rejected() {
    let wire = [0u8; TICK_SIZE - 1];
    assert!(Tick::decode(&wire).is_none(), "short record rejected");
    assert!(Tick::decode(&[]).is_none(), "empty record rejected");
}

#[test]
fn segment_append_replay_in_order() {
    let mut backing = [0u8; HEADER_SIZE + 8 * TICK_SIZE];
    let mut seg = must_some(SegmentBuf::init(&mut backing), "init");
    for i in 0..8u64 {
        assert!(seg.push(&tick(i)), "push {i} of 8 fits");
    }
    assert!(!seg.push(&tick(8)), "full segment rejects");
    assert_eq!(seg.count(), 8, "count tracked in the header");
    for (i, t) in seg.replay().enumerate() {
        assert_eq!(t.ts_ns, tick(i as u64).ts_ns, "replay order {i}");
        assert_eq!(t.bid_px, tick(i as u64).bid_px, "replay price {i}");
    }
}

#[test]
fn segment_open_validates_header() {
    // A foreign buffer (zeroed) must not open as a segment.
    let mut backing = [0u8; HEADER_SIZE + 4 * TICK_SIZE];
    assert!(
        SegmentBuf::open(&mut backing).is_none(),
        "zeroed buffer has no magic"
    );
    // A properly initialized segment re-opens.
    {
        let mut seg = must_some(SegmentBuf::init(&mut backing), "init");
        assert!(seg.push(&tick(1)), "one push fits");
    }
    let seg = must_some(SegmentBuf::open(&mut backing), "valid header re-opens");
    assert_eq!(seg.count(), 1, "count survives reopen");
    assert_eq!(
        seg.get(0).map(|t| t.ts_ns),
        Some(tick(1).ts_ns),
        "record survives reopen"
    );
}

#[test]
fn hostile_count_rejected_on_open() {
    let mut backing = [0u8; HEADER_SIZE + 2 * TICK_SIZE];
    {
        let mut seg = must_some(SegmentBuf::init(&mut backing), "init");
        assert!(seg.push(&tick(1)));
        assert!(seg.push(&tick(2)));
    }
    // Forge the count header to 99: declared extent past the buffer.
    let forged = 99u32;
    if let Some(slot) = backing.get_mut(12..16) {
        slot.copy_from_slice(&forged.to_le_bytes());
    }
    assert!(
        SegmentBuf::open(&mut backing).is_none(),
        "declared count past the buffer is rejected"
    );
}

#[test]
fn tiny_segment_rejected_at_init() {
    let mut tiny = [0u8; HEADER_SIZE + TICK_SIZE - 1];
    assert!(
        SegmentBuf::init(&mut tiny).is_none(),
        "a segment that cannot hold a record is not a segment"
    );
}
