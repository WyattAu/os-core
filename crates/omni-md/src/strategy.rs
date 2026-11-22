//! The tick-to-trade decision stage (pillar XXV's software form): a
//! pure, allocation-free strategy function over the decoded tick
//! stream. v0.1 is a deliberately simple spread rule — the pipeline
//! seams (ingest → decode → THIS → persist) are the design; the
//! strategy is swappable without touching them.

use crate::FRAME_LEN;
use omni_sbe::SbeDecoder;
use omni_tick::Tick;

/// A trade decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Sell into the printed bid (spread collapsed below threshold).
    Sell,
    /// Buy at the printed ask.
    Buy,
}

/// The maximum spread (in price ticks) that counts as collapsed.
pub const SPREAD_THRESHOLD: u64 = 2;

/// The trade decision for `frame`, given the previously seen bid.
/// `None` when the frame is not a valid tick (hostile input) or the
/// rule has no opinion (spread above the threshold).
///
/// The strategy reads only the decoded tick — it never sees the wire.
#[must_use]
pub fn decide(frame: &[u8], prev_bid: Option<u64>) -> Option<(Side, Tick)> {
    if frame.len() < FRAME_LEN {
        return None;
    }
    let d = SbeDecoder::new(frame);
    let _header = d.message_header()?;
    let ask = d.u64_at(24)?;
    let tick = Tick {
        instrument: d.u32_at(8)?,
        flags: d.u32_at(12)?,
        bid_px: d.u64_at(16)?,
        ask_px: ask,
        ts_ns: d.u64_at(32)?,
    };
    let side = match prev_bid {
        Some(prev) if ask < prev => Side::Sell,
        Some(prev) if tick.bid_px > prev => Side::Buy,
        _ => return None,
    };
    Some((side, tick))
}
