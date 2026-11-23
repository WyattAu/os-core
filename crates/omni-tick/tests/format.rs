//! omni-tick format tests: tick wire roundtrip, segment append/replay,
//! header validation, and hostile-extent rejection (REQ-OS-010).

use omni_tick::{SegmentBuf, Tick, CRC_SIZE, HEADER_SIZE, TICK_SIZE};
use proptest::prelude::*;

/// Fails unless `Some`, returning the value (house assert+abort style).
trait ExpectClean {
    type Value;
    fn expect_clean(self, what: &str) -> Self::Value;
}

impl<T> ExpectClean for Option<T> {
    type Value = T;
    fn expect_clean(self, what: &str) -> T {
        assert!(self.is_some(), "{what}");
        match self {
            Some(v) => v,
            None => std::process::abort(),
        }
    }
}

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
    let mut backing = [0u8; HEADER_SIZE + 8 * TICK_SIZE + CRC_SIZE];
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
    let mut backing = [0u8; HEADER_SIZE + 4 * TICK_SIZE + CRC_SIZE];
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
    let mut backing = [0u8; HEADER_SIZE + 2 * TICK_SIZE + CRC_SIZE];
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

#[test]
fn crc_corruption_rejected_on_open() {
    let mut backing = [0u8; HEADER_SIZE + 3 * TICK_SIZE + CRC_SIZE];
    {
        let mut seg = must_some(SegmentBuf::init(&mut backing), "init");
        for i in 0..3u64 {
            assert!(seg.push(&tick(i)), "push {i}");
        }
    }
    // Flip one byte inside the record region: the CRC must catch it.
    let record_byte = HEADER_SIZE + 5;
    flip_byte(&mut backing, record_byte);
    assert!(
        SegmentBuf::open(&mut backing).is_none(),
        "record corruption rejected"
    );
    // Heal and flip the TRAILER instead: still rejected.
    flip_byte(&mut backing, record_byte);
    let trailer_byte = backing.len() - 1;
    flip_byte(&mut backing, trailer_byte);
    assert!(
        SegmentBuf::open(&mut backing).is_none(),
        "trailer corruption rejected"
    );
    // Heal: the intact segment opens and replays.
    flip_byte(&mut backing, trailer_byte);
    let seg = must_some(SegmentBuf::open(&mut backing), "healed segment opens");
    assert_eq!(seg.count(), 3, "count intact");
    assert_eq!(
        seg.get(2).map(|t| t.ts_ns),
        Some(tick(2).ts_ns),
        "records intact"
    );
}

#[test]
fn capacity_accounts_for_the_trailer() {
    // A buffer sized exactly for header + 2 records + trailer holds
    // exactly 2 records — the trailer is not spare record space.
    let mut backing = [0u8; HEADER_SIZE + 2 * TICK_SIZE + CRC_SIZE];
    let mut seg = must_some(SegmentBuf::init(&mut backing), "init");
    assert!(seg.push(&tick(0)));
    assert!(seg.push(&tick(1)));
    assert!(!seg.push(&tick(2)), "trailer bytes are not record space");
}

/// Flips every bit of `buf[off]` (bounds-checked; off is in range by
/// construction at every call site).
fn flip_byte(buf: &mut [u8], off: usize) {
    if let Some(b) = buf.get_mut(off) {
        *b ^= 0xFF;
    }
}

// ---- Pillar LIV: monotonic invariant (530) + range queries (526). ----

#[test]
fn monotonic_push_enforces_strict_order() {
    let mut backing = [0u8; HEADER_SIZE + 4 * TICK_SIZE + CRC_SIZE];
    let mut seg = must_some(SegmentBuf::init(&mut backing), "init");
    assert!(seg.push_monotonic(&tick(0)).is_ok(), "first tick always in");
    assert!(seg.push_monotonic(&tick(1)).is_ok(), "advancing tick in");
    // Duplicate timestamp: rejected (strictly greater required).
    assert_eq!(
        seg.push_monotonic(&tick(1)),
        Err(omni_tick::PushError::OutOfOrder),
        "duplicate ts rejected"
    );
    // Descending: rejected.
    assert_eq!(
        seg.push_monotonic(&tick(0)),
        Err(omni_tick::PushError::OutOfOrder),
        "out-of-order rejected"
    );
    assert_eq!(seg.count(), 2, "rejected ticks are not stored");
    assert_eq!(seg.last_ts(), Some(tick(1).ts_ns), "last anchor tracks");
}

#[test]
fn monotonic_full_is_full_error() {
    let mut backing = [0u8; HEADER_SIZE + 2 * TICK_SIZE + CRC_SIZE];
    let mut seg = must_some(SegmentBuf::init(&mut backing), "init");
    assert!(seg.push_monotonic(&tick(0)).is_ok());
    assert!(seg.push_monotonic(&tick(1)).is_ok());
    assert_eq!(
        seg.push_monotonic(&tick(2)),
        Err(omni_tick::PushError::Full),
        "capacity exhaustion reports Full, not OutOfOrder"
    );
}

#[test]
fn range_query_binary_search_over_monotonic_segment() {
    let mut backing = [0u8; HEADER_SIZE + 8 * TICK_SIZE + CRC_SIZE];
    let mut seg = must_some(SegmentBuf::init(&mut backing), "init");
    for i in 0..8u64 {
        let mut t = tick(i);
        t.ts_ns = 1000 + i * 100; // strictly ascending
        assert!(seg.push_monotonic(&t).is_ok(), "push {i}");
    }
    // [1100, 1400): records 1, 2, 3.
    let in_range: Vec<u64> = seg.range(1100, 1400).map(|t| t.ts_ns).collect();
    assert_eq!(in_range, vec![1100, 1200, 1300], "half-open range");
    // Empty range: end before start.
    assert!(
        seg.range(1400, 1100).next().is_none(),
        "inverted range empty"
    );
    // Boundary: start == a record's ts includes it; end excludes it.
    assert_eq!(
        seg.range(1000, 1000).count(),
        0,
        "empty at zero-width range"
    );
    // Past-the-end range is empty.
    assert!(seg.range(9900, 9999).next().is_none(), "beyond the data");
    // Full range returns everything.
    assert_eq!(seg.range(0, u64::MAX).count(), 8, "full range = replay");
}

// ---- Pillar LIV goal 528: tick-to-bar synthesis. ----

use omni_tick::bars::BarAggregator;

fn quote(ts: u64, bid: u64) -> Tick {
    Tick {
        ts_ns: ts,
        instrument: 1,
        flags: 0,
        bid_px: bid,
        ask_px: bid + 1,
    }
}

#[test]
fn bars_complete_at_epoch_crossings() {
    // bucket 100 ns; ticks at 50, 150, 160, 300 → bars for epochs
    // 0, 100, 300 (the 200-epoch has no ticks: no bar).
    let mut agg = BarAggregator::new(100).expect_clean("nonzero bucket");
    let feed = [quote(50, 10), quote(150, 12), quote(160, 11), quote(300, 9)];
    let mut bars = Vec::new();
    for q in &feed {
        if let Some(b) = agg.push(q) {
            bars.push(b);
        }
    }
    assert_eq!(bars.len(), 2, "epochs 0 and 100 completed; 300 open");
    let b0 = bars.first().expect_clean("epoch-0 bar");
    assert_eq!(b0.epoch_start_ns, 0);
    assert_eq!(b0.open, 10);
    assert_eq!(b0.close, 10);
    assert!(b0.bounds_hold(), "bounds invariant");
    let b1 = bars.get(1).expect_clean("epoch-100 bar");
    assert_eq!(b1.epoch_start_ns, 100);
    assert_eq!(b1.open, 12);
    assert_eq!(b1.high, 12);
    assert_eq!(b1.low, 11);
    assert_eq!(b1.close, 11);
    assert_eq!(b1.volume, 2, "two ticks in epoch 100");
}

#[test]
fn flush_emits_the_open_bar() {
    let mut agg = BarAggregator::new(100).expect_clean("nonzero bucket");
    assert!(agg.push(&quote(50, 10)).is_none(), "open bar is not done");
    let flushed = agg.push(&quote(60, 11));
    assert!(flushed.is_none(), "same epoch: still open");
    let done = agg.flush().expect_clean("flush emits the open bar");
    assert_eq!(done.epoch_start_ns, 0);
    assert_eq!(done.open, 10);
    assert_eq!(done.close, 11);
    assert!(done.bounds_hold());
    // The aggregator is empty afterwards.
    assert!(agg.flush().is_none(), "flush on empty is None");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The bounds invariant survives ANY tick stream: whatever the
    /// epoch layout, every emitted bar's prices stay inside its
    /// [low, high].
    #[test]
    fn bar_bounds_hold_under_drift(
        ts in proptest::collection::vec(0u64..1000, 0..40),
        price in proptest::collection::vec(proptest::arbitrary::any::<u64>(), 0..40),
    ) {
        let mut agg = BarAggregator::new(50).expect_clean("nonzero bucket");
        for (ts, price) in ts.into_iter().zip(price) {
            if let Some(b) = agg.push(&quote(ts, price)) {
                assert!(b.bounds_hold(), "bounds invariant per bar");
            }
        }
        if let Some(b) = agg.flush() {
            assert!(b.bounds_hold(), "flushed bar bounds");
        }
    }
}
