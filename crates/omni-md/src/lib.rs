//! # omni-md — the market-data pipeline (ADR-0017, server half)
//!
//! The seams of the HFT data plane: an SBE frame in (omni-sbe), a
//! tick out (omni-tick), fan-out through the descriptor ring
//! (omni-ring). Ingestion (omni-io socket reads) and the HTTP query
//! surface (omni-service posture) plug into these seams in later
//! tranches; this crate fixes the contracts between them.
//!
//! Posture: `no_std`, zero `unsafe`, zero allocation, zero panics —
//! untrusted wire input is `Option`/`bool`-rejected, never crashed on.

#![no_std]

pub mod fanout;
pub mod strategy;

use omni_sbe::SbeDecoder;
use omni_tick::Tick;

/// The trade-tick template id of our minimal feed schema.
pub const TICK_TEMPLATE_ID: u16 = 1;
/// The schema id the feed carries.
pub const TICK_SCHEMA_ID: u16 = 1;
/// Frame length: 8-byte SBE header + 32-byte tick body.
pub const FRAME_LEN: usize = 40;
/// The body's block length as declared in the header.
pub const BODY_BLOCK_LEN: u16 = 32;

/// Field offsets inside the frame (after the 8-byte SBE header).
mod body {
    pub const INSTRUMENT: usize = 8;
    pub const FLAGS: usize = 12;
    pub const BID: usize = 16;
    pub const ASK: usize = 24;
    pub const TS: usize = 32;
}

/// Decodes one trade-tick frame. The SBE header is validated (schema
/// and template must be ours; block length and frame length must
/// cover the body) and the body scalars are read bounds-checked —
/// untrusted wire input is rejected, never wrapped.
#[must_use]
pub fn frame_to_tick(frame: &[u8]) -> Option<Tick> {
    if frame.len() < FRAME_LEN {
        return None;
    }
    let d = SbeDecoder::new(frame);
    let h = d.message_header()?;
    if h.schema_id != TICK_SCHEMA_ID || h.template_id != TICK_TEMPLATE_ID {
        return None;
    }
    if h.block_length != BODY_BLOCK_LEN {
        return None;
    }
    Some(Tick {
        instrument: d.u32_at(body::INSTRUMENT)?,
        flags: d.u32_at(body::FLAGS)?,
        bid_px: d.u64_at(body::BID)?,
        ask_px: d.u64_at(body::ASK)?,
        ts_ns: d.u64_at(body::TS)?,
    })
}
