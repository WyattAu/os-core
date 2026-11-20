//! # omni-tick — the tick store's first design (ADR-0017)
//!
//! A fixed-width 32-byte tick record and an append-only segment
//! container: 16-byte header (magic, version, record size, count)
//! followed by records, replay = sequential scan, no WAL and no LSM —
//! the HFT discipline is append + scan, and the store's contract is
//! replay, not OLTP.
//!
//! Posture: `no_std`, zero `unsafe`, zero allocation, zero panics —
//! every out-of-bounds access is an `Option`/`bool`, never a crash.
//! The wire format is little-endian and fixed at v1: omni-md, the
//! FPGA front-end, and any replay tool compile against these 32 bytes.

#![no_std]

/// Segment container magic: `OMTICK\0\0`.
pub const MAGIC: &[u8; 8] = b"OMTICK\0\0";
/// Wire-format version this crate writes and accepts.
pub const FORMAT_VERSION: u16 = 1;
/// Record size in bytes (fixed at 32 by ADR-0017).
pub const TICK_SIZE: usize = 32;
/// Segment header size in bytes (magic 8, version 2, record size 2,
/// count 4).
pub const HEADER_SIZE: usize = 16;

/// One market-data tick, fixed width, little-endian on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// Exchange timestamp, nanoseconds since the epoch.
    pub ts_ns: u64,
    /// Instrument id (the feed's security index).
    pub instrument: u32,
    /// Side/condition flags (feed-defined bit set).
    pub flags: u32,
    /// Best bid, fixed-point price (feed-defined scale).
    pub bid_px: u64,
    /// Best ask, fixed-point price (feed-defined scale).
    pub ask_px: u64,
}

/// LE `u64` into `dst[off..off+8]`; ignored when out of bounds.
fn put_u64(dst: &mut [u8], off: usize, v: u64) {
    if let Some(slot) = dst.get_mut(off..off.saturating_add(8)) {
        slot.copy_from_slice(&v.to_le_bytes());
    }
}

/// LE `u32` into `dst[off..off+4]`; ignored when out of bounds.
fn put_u32(dst: &mut [u8], off: usize, v: u32) {
    if let Some(slot) = dst.get_mut(off..off.saturating_add(4)) {
        slot.copy_from_slice(&v.to_le_bytes());
    }
}

/// LE `u16` into `dst[off..off+2]`; ignored when out of bounds.
fn put_u16(dst: &mut [u8], off: usize, v: u16) {
    if let Some(slot) = dst.get_mut(off..off.saturating_add(2)) {
        slot.copy_from_slice(&v.to_le_bytes());
    }
}

/// LE `u64` from `src[off..off+8]`; `None` when out of bounds.
fn get_u64(src: &[u8], off: usize) -> Option<u64> {
    let b = src.get(off..off.saturating_add(8))?;
    Some(u64::from_le_bytes(<[u8; 8]>::try_from(b).ok()?))
}

/// LE `u32` from `src[off..off+4]`; `None` when out of bounds.
fn get_u32(src: &[u8], off: usize) -> Option<u32> {
    let b = src.get(off..off.saturating_add(4))?;
    Some(u32::from_le_bytes(<[u8; 4]>::try_from(b).ok()?))
}

/// LE `u16` from `src[off..off+2]`; `None` when out of bounds.
fn get_u16(src: &[u8], off: usize) -> Option<u16> {
    let b = src.get(off..off.saturating_add(2))?;
    Some(u16::from_le_bytes(<[u8; 2]>::try_from(b).ok()?))
}

impl Tick {
    /// Wire-encodes the tick into `dst` (exactly [`TICK_SIZE`] bytes).
    /// Out-of-bounds writes are dropped silently — callers pass the
    /// right-sized record slot.
    pub fn encode(&self, dst: &mut [u8]) {
        put_u64(dst, 0, self.ts_ns);
        put_u32(dst, 8, self.instrument);
        put_u32(dst, 12, self.flags);
        put_u64(dst, 16, self.bid_px);
        put_u64(dst, 24, self.ask_px);
    }

    /// Decodes a wire tick from `src` (exactly [`TICK_SIZE`] bytes).
    /// `None` when the slice is short — untrusted wire input is
    /// rejected, never wrapped.
    #[must_use]
    pub fn decode(src: &[u8]) -> Option<Self> {
        if src.len() < TICK_SIZE {
            return None;
        }
        Some(Self {
            ts_ns: get_u64(src, 0)?,
            instrument: get_u32(src, 8)?,
            flags: get_u32(src, 12)?,
            bid_px: get_u64(src, 16)?,
            ask_px: get_u64(src, 24)?,
        })
    }
}

/// An append-only segment over a caller-owned byte buffer: the
/// durability contract is "write once, replay in order". The buffer is
/// typically an mmap'd page (owned by omni-md later); this type only
/// enforces the format.
pub struct SegmentBuf<'a> {
    buf: &'a mut [u8],
}

impl<'a> SegmentBuf<'a> {
    /// Wraps an empty buffer and writes the segment header. Returns
    /// `None` when the buffer cannot hold a header plus one record —
    /// a segment that cannot store a tick is not a segment.
    pub fn init(buf: &'a mut [u8]) -> Option<Self> {
        if buf.len() < HEADER_SIZE + TICK_SIZE {
            return None;
        }
        put_u32(buf, 0, 0); // magic low half
        put_u32(buf, 4, 0); // magic high half (written below)
        let magic_words: [u32; 2] = [
            u32::from_le_bytes([MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3]]),
            u32::from_le_bytes([MAGIC[4], MAGIC[5], MAGIC[6], MAGIC[7]]),
        ];
        put_u32(buf, 0, magic_words[0]);
        put_u32(buf, 4, magic_words[1]);
        put_u16(buf, 8, FORMAT_VERSION);
        put_u16(buf, 10, u16::try_from(TICK_SIZE).unwrap_or_default());
        put_u32(buf, 12, 0); // count
        Some(Self { buf })
    }

    /// Wraps an existing populated segment (e.g. an mmap'd file page)
    /// and validates its header. `None` on a foreign/malformed header.
    #[must_use]
    pub fn open(buf: &'a mut [u8]) -> Option<Self> {
        if buf.len() < HEADER_SIZE {
            return None;
        }
        let m0 = get_u32(buf, 0)?;
        let m1 = get_u32(buf, 4)?;
        let magic_words: [u32; 2] = [
            u32::from_le_bytes([MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3]]),
            u32::from_le_bytes([MAGIC[4], MAGIC[5], MAGIC[6], MAGIC[7]]),
        ];
        if m0 != magic_words[0] || m1 != magic_words[1] {
            return None;
        }
        if get_u16(buf, 8)? != FORMAT_VERSION
            || get_u16(buf, 10)? != u16::try_from(TICK_SIZE).unwrap_or_default()
        {
            return None;
        }
        let count = get_u32(buf, 12)?;
        if HEADER_SIZE + count as usize * TICK_SIZE > buf.len() {
            return None; // declared count past the buffer: hostile
        }
        Some(Self { buf })
    }

    /// Appends a tick. `false` when the segment is full — callers
    /// roll to the next segment (the store's rollover policy).
    pub fn push(&mut self, tick: &Tick) -> bool {
        let count = self.count() as usize;
        let Some(total) = (count + 1).checked_mul(TICK_SIZE) else {
            return false;
        };
        let Some(end) = HEADER_SIZE.checked_add(total) else {
            return false;
        };
        if end > self.buf.len() {
            return false;
        }
        let slot = HEADER_SIZE + count * TICK_SIZE;
        if let Some(rec) = self.buf.get_mut(slot..end) {
            tick.encode(rec);
        }
        put_u32(self.buf, 12, u32::try_from(count + 1).unwrap_or_default());
        true
    }

    /// The segment's record count (from the header).
    #[must_use]
    pub fn count(&self) -> u32 {
        get_u32(self.buf, 12).unwrap_or(0)
    }

    /// Reads record `i`; `None` when out of range.
    #[must_use]
    pub fn get(&self, i: u32) -> Option<Tick> {
        if i >= self.count() {
            return None;
        }
        let off = HEADER_SIZE + i as usize * TICK_SIZE;
        let rec = self.buf.get(off..off + TICK_SIZE)?;
        Tick::decode(rec)
    }

    /// Replay: every record in order.
    pub fn replay(&self) -> impl Iterator<Item = Tick> + '_ {
        let count = self.count();
        (0..count).filter_map(move |i| self.get(i))
    }
}
