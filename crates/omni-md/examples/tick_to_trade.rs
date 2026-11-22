//! Tick-to-trade demo (Phase 2, ADR-0009's exit criterion): the WHOLE
//! data plane per tick — omni-sbe encode (wire) → omni-md decode →
//! strategy decide → omni-tick persist — with per-tick latency
//! captured around the hot section.
//!
//! This is a demo, not a benchmark (`make bench` owns numbers): the
//! point is that every seam of ADR-0017's plane runs in one process
//! with no allocation on the hot path.
//!
//! Run: `cargo run --example tick_to_trade -p omni-md -- 256`

use std::time::Instant;

use omni_md::fanout::Fanout;
use omni_md::strategy::decide;
use omni_md::{frame_to_tick, FRAME_LEN, TICK_SCHEMA_ID, TICK_TEMPLATE_ID};
use omni_sbe::SbeEncoder;
use omni_tick::{SegmentBuf, Tick, CRC_SIZE, TICK_SIZE};

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(256);

    let mut page = vec![0u8; omni_tick::HEADER_SIZE + n * TICK_SIZE + CRC_SIZE];
    let Some(mut segment) = SegmentBuf::init(&mut page) else {
        eprintln!("segment init failed");
        std::process::abort();
    };
    let mut fan: Fanout<64> = Fanout::new();

    let mut prev_bid: Option<u64> = None;
    let mut latencies = Vec::with_capacity(n);
    let mut trades = 0usize;
    let mut wire = [0u8; FRAME_LEN];

    for i in 0..n {
        // ---- wire: the feed's SBE frame (omni-sbe encode) ----
        let bid: u64 =
            4_295_032_500 + u64::try_from(if i % 3 == 0 { i } else { 0 }).unwrap_or_default();
        let ask: u64 = bid + 2 - u64::from(i % 3 == 2);
        let tick_wire = Tick {
            ts_ns: 1_700_000_000_000_000_000 + i as u64 * 100,
            instrument: u32::try_from(i % 4).unwrap_or_default(),
            flags: 0,
            bid_px: bid,
            ask_px: ask.max(bid),
        };
        let mut enc = SbeEncoder::new(&mut wire);
        enc.write_header(32, TICK_TEMPLATE_ID, TICK_SCHEMA_ID, 0);
        let body_ok = enc.put_u32(8, tick_wire.instrument)
            && enc.put_u32(12, tick_wire.flags)
            && enc.put_u64(16, tick_wire.bid_px)
            && enc.put_u64(24, tick_wire.ask_px)
            && enc.put_u64(32, tick_wire.ts_ns);
        if !body_ok {
            eprintln!("encode failed at {i}");
            std::process::abort();
        }

        // ---- the hot section: decode → decide → persist ----
        let t0 = Instant::now();
        let Some(tick) = frame_to_tick(&wire) else {
            eprintln!("decode failed at {i}");
            std::process::abort();
        };
        let decision = decide(&wire, prev_bid);
        prev_bid = Some(tick.bid_px); // the anchor moves every tick
        let persisted = match decision {
            Some((side, t)) => {
                let _ = side;
                segment.push(&t)
            }
            None => segment.push(&tick),
        };
        let _ = fan.publish(i as u64, &tick);
        let _ = fan.drain_one();
        let elapsed = t0.elapsed();
        // ---- hot section ends ----

        if !persisted {
            eprintln!("persist failed at {i}");
            std::process::abort();
        }
        if let Some((side, _)) = decision {
            trades += 1;
            println!("tick {i:4}: {:4?} at bid {}", side, tick_wire.bid_px);
        }
        latencies.push(elapsed);
    }

    latencies.sort_unstable();
    // Percentile index: q in [0, 1] keeps the product in range; the
    // min() below clamps the tail case — truncation is the point.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let pick = |q: f64| -> u128 {
        let idx = ((latencies.len() as f64) * q).floor() as usize;
        latencies
            .get(idx.min(latencies.len().saturating_sub(1)))
            .map(core::time::Duration::as_nanos)
            .unwrap_or_default()
    };
    println!();
    println!("== tick-to-trade over {n} frames ==");
    println!(
        "decide+persist: min {:>4} ns | p50 {:>4} ns | p99 {:>4} ns | max {:>4} ns",
        pick(0.0),
        pick(0.50),
        pick(0.99),
        latencies
            .last()
            .map(core::time::Duration::as_nanos)
            .unwrap_or_default(),
    );
    println!(
        "trades: {trades} | stored: {} | fan-out drained: {}",
        segment.count(),
        n
    );
}
