//! The HFT data plane's HTTP surface (ADR-0017): the omni-md pipeline
//! feeding an omni-tick segment, served over the omni-service axum
//! posture.
//!
//! Boot: `--seed N` (default 16) synthetic ticks travel the WHOLE
//! plane in-process — omni-sbe encode → omni-md decode → fan-out →
//! omni-tick segment — proving every seam before a real feed exists.
//! Endpoints: `/healthz`, `/readyz`, `/ticks/latest?n=K`,
//! `/stats`. The real UDP feed replaces the synthetic seeder in the
//! hardware-session tranche; the route surface is the contract.
//!
//! The JSON projection is hand-rolled (five fixed fields, no serde
//! derive) — one less dependency on the latency path; the field set
//! is pinned by ADR-0017's record format.
//!
//! Coverage (ADR-0005): this binary is the process surface —
//! exercised by the spawned-binary smoke test, excluded from the
//! line-coverage gate like omni-service's.

use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::routing::get;
use axum::Router;
use omni_md::{fanout::Fanout, frame_to_tick, FRAME_LEN, TICK_SCHEMA_ID, TICK_TEMPLATE_ID};
use omni_sbe::SbeEncoder;
use omni_tick::{SegmentBuf, Tick, HEADER_SIZE, TICK_SIZE};

/// Segment capacity in records (v0.1: one fixed segment).
const SEGMENT_RECORDS: usize = 4096;

/// Shared store: the segment + the pipeline's fan-out (drained at
/// seed time; the live feed drains on its own task in a later tranche).
struct Store {
    segment: Mutex<SegmentBuf<'static>>,
    seeded: usize,
}

/// Synthetic seed: encode a tick to the wire, decode it back through
/// the pipeline, publish through the fan-out, persist to the segment.
/// Every seam of ADR-0017's data plane, exercised in order.
fn seed_plane(fan: &mut Fanout<64>, segment: &mut SegmentBuf<'_>, n: usize) -> usize {
    let mut seeded = 0usize;
    for i in 0..n {
        let tick = Tick {
            ts_ns: 1_700_000_000_000_000_000 + i as u64 * 100,
            instrument: u32::try_from(i % 8).unwrap_or_default(),
            flags: 0,
            bid_px: 4_295_032_500 + i as u64,
            ask_px: 4_295_033_000 + i as u64,
        };
        let mut frame = [0u8; FRAME_LEN];
        let mut enc = SbeEncoder::new(&mut frame);
        enc.write_header(32, TICK_TEMPLATE_ID, TICK_SCHEMA_ID, 0);
        let body_ok = enc.put_u32(8, tick.instrument)
            && enc.put_u32(12, tick.flags)
            && enc.put_u64(16, tick.bid_px)
            && enc.put_u64(24, tick.ask_px)
            && enc.put_u64(32, tick.ts_ns);
        if !body_ok {
            continue;
        }
        let Some(decoded) = frame_to_tick(&frame) else {
            continue;
        };
        let seq = i as u64;
        if !fan.publish(seq, &decoded) {
            continue;
        }
        if let Some(dran) = fan.drain_one() {
            if segment.push(&dran) {
                seeded += 1;
            }
        }
    }
    seeded
}

fn tick_json(t: &Tick) -> String {
    format!(
        "{{\"ts_ns\":{},\"instrument\":{},\"flags\":{},\"bid_px\":{},\"ask_px\":{}}}",
        t.ts_ns, t.instrument, t.flags, t.bid_px, t.ask_px
    )
}

fn json_response(body: String) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(store): State<Arc<Store>>) -> &'static str {
    let guard = store
        .segment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = guard.count();
    "ready"
}

async fn ticks_latest(
    State(store): State<Arc<Store>>,
    Path(n): Path<String>,
) -> axum::response::Response {
    let guard = store
        .segment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let count = guard.count();
    let n = n.parse::<u32>().unwrap_or(1).min(count).min(256);
    let start = count.saturating_sub(n);
    let mut body = String::from("[");
    for i in start..count {
        let Some(t) = guard.get(i) else { continue };
        if i > start {
            body.push(',');
        }
        body.push_str(&tick_json(&t));
    }
    body.push(']');
    json_response(body)
}

async fn stats(State(store): State<Arc<Store>>) -> axum::response::Response {
    let guard = store
        .segment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    json_response(format!(
        "{{\"records\":{},\"capacity\":{},\"seeded\":{}}}",
        guard.count(),
        SEGMENT_RECORDS,
        store.seeded
    ))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = std::env::var("OMNI_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8090);
    let seed_count: usize = std::env::var("OMNI_TICKS")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(16);

    // The segment's backing page is process-lifetime: leak one boxed
    // buffer and hand the store the 'static slice.
    let page = Box::leak(vec![0u8; HEADER_SIZE + SEGMENT_RECORDS * TICK_SIZE].into_boxed_slice());
    let Some(mut segment) = SegmentBuf::init(page) else {
        return Err("segment init failed".into());
    };
    let mut fan: Fanout<64> = Fanout::new();
    let seeded = seed_plane(&mut fan, &mut segment, seed_count);

    let store = Arc::new(Store {
        segment: Mutex::new(segment),
        seeded,
    });
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/ticks/latest/{n}", get(ticks_latest))
        .route("/stats", get(stats))
        .with_state(store);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
