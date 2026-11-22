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

/// Model-check harnesses (`cargo kani`) — local gate only.
pub mod gorilla;

#[cfg(kani)]
mod kani;

/// Segment container magic: `OMTICK\0\0`.
pub const MAGIC: &[u8; 8] = b"OMTICK\0\0";
/// Wire-format version this crate writes and accepts.
pub const FORMAT_VERSION: u16 = 2;
/// Record size in bytes (fixed at 32 by ADR-0017).
pub const TICK_SIZE: usize = 32;
/// Segment header size in bytes (magic 8, version 2, record size 2,
/// count 4).
pub const HEADER_SIZE: usize = 16;
/// CRC32 trailer size in bytes (the segment's last 4).
pub const CRC_SIZE: usize = 4;

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
    /// Streaming CRC32 of the record region (records only, not the
    /// header or the trailer) — updated per push, published in the
    /// trailer at every write.
    crc: crc32fast::Hasher,
}

impl<'a> SegmentBuf<'a> {
    /// The segment's record capacity for a buffer of `len` bytes:
    /// header + records + CRC trailer must all fit.
    #[must_use]
    pub fn capacity_for(len: usize) -> usize {
        len.saturating_sub(HEADER_SIZE + CRC_SIZE) / TICK_SIZE
    }

    /// Wraps an empty buffer and writes the segment header. Returns
    /// `None` when the buffer cannot hold a header, one record, and
    /// the CRC trailer — a segment that cannot store a tick is not a
    /// segment.
    pub fn init(buf: &'a mut [u8]) -> Option<Self> {
        if buf.len() < HEADER_SIZE + CRC_SIZE + TICK_SIZE {
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
        put_u32(buf, buf.len() - CRC_SIZE, 0); // CRC32 of zero records
        Some(Self {
            buf,
            crc: crc32fast::Hasher::new(),
        })
    }

    /// Wraps an existing populated segment (e.g. an mmap'd file page)
    /// and validates its header AND CRC trailer. `None` on a
    /// foreign/malformed header, a declared count past the buffer, or
    /// a CRC mismatch — bit rot and truncation are rejected, never
    /// replayed.
    #[must_use]
    pub fn open(buf: &'a mut [u8]) -> Option<Self> {
        if buf.len() < HEADER_SIZE + CRC_SIZE + TICK_SIZE {
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
        let records_len = usize::try_from(count)
            .ok()
            .and_then(|c| c.checked_mul(TICK_SIZE))?;
        let end = HEADER_SIZE.checked_add(records_len)?;
        if end + CRC_SIZE > buf.len() {
            return None; // declared count past the buffer: hostile
        }
        // CRC trailer: the buffer's last 4 bytes, over the record
        // region only.
        let stored = get_u32(buf, buf.len() - CRC_SIZE)?;
        let mut crc = crc32fast::Hasher::new();
        if let Some(region) = buf.get(HEADER_SIZE..end) {
            crc.update(region);
        }
        if crc.clone().finalize() != stored {
            return None;
        }
        Some(Self { buf, crc })
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
        if end + CRC_SIZE > self.buf.len() {
            return false;
        }
        let slot = HEADER_SIZE + count * TICK_SIZE;
        if let Some(rec) = self.buf.get_mut(slot..end) {
            tick.encode(rec);
            // Streaming CRC: the trailer always reflects every record.
            self.crc.update(rec);
        }
        put_u32(self.buf, 12, u32::try_from(count + 1).unwrap_or_default());
        put_u32(
            self.buf,
            self.buf.len() - CRC_SIZE,
            self.crc.clone().finalize(),
        );
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

    /// The timestamp of the last record in the segment; `None` when
    /// the segment is empty (pillar LIV goal 530's ordering anchor).
    #[must_use]
    pub fn last_ts(&self) -> Option<u64> {
        self.get(self.count().checked_sub(1)?).map(|t| t.ts_ns)
    }

    /// The raw timestamp of record `i` (header offset 0 of the
    /// record); `None` when out of range. O(1) — no decode.
    fn ts_at(&self, i: u32) -> Option<u64> {
        let off = HEADER_SIZE + i as usize * TICK_SIZE;
        get_u64(self.buf, off)
    }

    /// Appends a tick, enforcing pillar LIV goal 530's monotonic
    /// timestamp invariant: the tick's `ts_ns` must be strictly
    /// greater than the last record's. Out-of-order and duplicate
    /// timestamps are rejected — the store never reorders on the
    /// caller's behalf (goal 530's tie-break injection is an ingress
    /// concern, not a store concern).
    ///
    /// # Errors
    ///
    /// [`PushError::OutOfOrder`] for a non-advancing timestamp;
    /// [`PushError::Full`] when the segment is at capacity.
    pub fn push_monotonic(&mut self, tick: &Tick) -> Result<(), PushError> {
        if let Some(last) = self.last_ts() {
            if tick.ts_ns <= last {
                return Err(PushError::OutOfOrder);
            }
        }
        if !self.push(tick) {
            return Err(PushError::Full);
        }
        Ok(())
    }

    /// Replay of the records whose timestamps fall in
    /// `[ts_start, ts_end)` — O(log n) slice bounds via binary search
    /// over the monotonic record order (pillar LIV goal 526's
    /// pointer-arithmetic reduction).
    ///
    /// Meaningful only for segments built with [`SegmentBuf::push`]
    /// after monotonic feeding (ascending `ts_ns`); a non-monotonic
    /// segment's range output is unspecified.
    pub fn range(&self, ts_start: u64, ts_end: u64) -> impl Iterator<Item = Tick> + '_ {
        // Binary search the monotonic region for the slice bounds.
        let start_idx = self.partition_point_ts(ts_start);
        let end_idx = self.partition_point_ts(ts_end);
        let count = self.count();
        let lo = start_idx.min(count);
        let hi = end_idx.min(count);
        ((lo)..hi).filter_map(move |i| self.get(i))
    }

    /// First record index whose timestamp is >= `ts` (partition
    /// point over the monotonic record order).
    fn partition_point_ts(&self, ts: u64) -> u32 {
        let count = self.count();
        let mut lo = 0u32;
        let mut hi = count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.ts_at(mid) {
                Some(t) if t < ts => lo = mid + 1,
                _ => hi = mid,
            }
        }
        lo
    }

    /// Replay: every record in order.
    pub fn replay(&self) -> impl Iterator<Item = Tick> + '_ {
        let count = self.count();
        (0..count).filter_map(move |i| self.get(i))
    }
}

/// [`SegmentBuf::push_monotonic`] failure kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    /// The tick's timestamp does not advance past the last record
    /// (pillar LIV goal 530: out-of-order and duplicate timestamps
    /// are rejected, never reordered).
    OutOfOrder,
    /// The segment is at capacity.
    Full,
}
