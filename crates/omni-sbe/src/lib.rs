//! # omni-sbe — zero-copy SBE frame decoder (pillar 208, CPU fallback)
//!
//! Decodes [SBE](https://www.fixtrading.org/standards/sbe/) (Simple Binary
//! Encoding) market-data frames in place: the decoder borrows the frame
//! buffer and hands out typed views, so ingestion never copies or
//! allocates. This is the portable baseline for pillar 208's branchless
//! AVX-512 extraction — the SIMD gather is a later optimization over the
//! same contract.
//!
//! Posture: `no_std`, zero `unsafe`, zero allocation, zero panics — every
//! out-of-bounds access is an `Option`/`Result`, never a crash. The HFT
//! ingestion path runs untrusted bytes; the decoder treats every length
//! and offset as hostile.
//!
//! Scope (v0.1.0): the message header, fixed-width scalar fields,
//! repeating-group iteration, and varData fields. The schema compiler
//! (DSL → typed codecs) is pillar VI/K6 territory and deliberately out
//! of scope here.

#![no_std]

/// SBE wire-format constants.
pub mod consts {
    /// The message header is always 8 bytes: blockLength, templateId,
    /// schemaId, version — each `u16` little-endian.
    pub const HEADER_LEN: usize = 8;
    /// A repeating-group dimensions header: blockLength + numInGroup.
    pub const GROUP_HEADER_LEN: usize = 4;
    /// varData length prefix: one `u8` in SBE v1.0.
    pub const VARLEN_PREFIX_LEN: usize = 1;
}

/// Every decode failure mode. No panics: untrusted frames are the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbeError {
    /// The frame is shorter than the operation's fixed extent.
    Truncated,
    /// A varData length prefix ran past the frame.
    VarLenOverflow,
    /// A repeating-group entry extent ran past the frame.
    GroupOverflow,
}

/// The SBE message header: every frame starts with these four `u16`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    /// Bytes in THIS message body (excluding the 8-byte header).
    pub block_length: u16,
    /// The schema template this frame decodes against.
    pub template_id: u16,
    /// The schema version this frame encodes.
    pub schema_id: u16,
    /// The template version this frame was written with.
    pub version: u16,
}

/// A zero-copy view over one SBE frame. All access is bounds-checked;
/// nothing in the frame is trusted.
#[derive(Debug)]
pub struct SbeDecoder<'a> {
    frame: &'a [u8],
}

impl<'a> SbeDecoder<'a> {
    /// Wraps a frame buffer. The frame is not copied and not mutated.
    #[must_use]
    pub fn new(frame: &'a [u8]) -> Self {
        Self { frame }
    }

    /// The whole frame length (header + body).
    #[must_use]
    pub const fn frame_len(&self) -> usize {
        self.frame.len()
    }

    /// Decodes the message header. Returns `None` if the frame is
    /// shorter than the 8-byte header.
    #[must_use]
    pub fn message_header(&self) -> Option<MessageHeader> {
        if self.frame.len() < consts::HEADER_LEN {
            return None;
        }
        let u16_at = |off: usize| {
            let a = self.frame.get(off).copied().unwrap_or(0);
            let b = self.frame.get(off + 1).copied().unwrap_or(0);
            u16::from_le_bytes([a, b])
        };
        Some(MessageHeader {
            block_length: u16_at(0),
            template_id: u16_at(2),
            schema_id: u16_at(4),
            version: u16_at(6),
        })
    }

    /// Reads a little-endian scalar of `N` bytes at `offset`.
    /// Returns `None` when the field extent runs past the frame.
    #[must_use]
    pub fn scalar<const N: usize>(&self, offset: usize) -> Option<[u8; N]> {
        let end = offset.checked_add(N)?;
        if end > self.frame.len() {
            return None;
        }
        let mut out = [0u8; N];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.frame.get(offset + i).copied().unwrap_or(0);
        }
        Some(out)
    }

    /// `u16` field at `offset` (little-endian).
    #[must_use]
    pub fn u16_at(&self, offset: usize) -> Option<u16> {
        self.scalar::<2>(offset).map(u16::from_le_bytes)
    }

    /// `u32` field at `offset` (little-endian).
    #[must_use]
    pub fn u32_at(&self, offset: usize) -> Option<u32> {
        self.scalar::<4>(offset).map(u32::from_le_bytes)
    }

    /// `u64` field at `offset` (little-endian).
    #[must_use]
    pub fn u64_at(&self, offset: usize) -> Option<u64> {
        self.scalar::<8>(offset).map(u64::from_le_bytes)
    }

    /// `i64` field at `offset` (little-endian).
    #[must_use]
    pub fn i64_at(&self, offset: usize) -> Option<i64> {
        self.scalar::<8>(offset).map(i64::from_le_bytes)
    }

    /// Iterates a repeating group that starts at `offset`: returns the
    /// group's `numInGroup` (the `u16` at `offset + 2` in the SBE v1.0
    /// dimensions header) and the per-entry `block_length` (the `u16`
    /// at `offset`). Entries begin at `offset + 4`.
    ///
    /// Returns `None` when the dimensions header or the first entry
    /// would run past the frame.
    #[must_use]
    pub fn group_at(&self, offset: usize) -> Option<GroupView<'a>> {
        if offset + consts::GROUP_HEADER_LEN > self.frame.len() {
            return None;
        }
        let byte = |off: usize| self.frame.get(off).copied();
        let (bl_lo, bl_hi, cnt_lo, cnt_hi) = (
            byte(offset)?,
            byte(offset + 1)?,
            byte(offset + 2)?,
            byte(offset + 3)?,
        );
        let block_length = u16::from_le_bytes([bl_lo, bl_hi]) as usize;
        let count = u16::from_le_bytes([cnt_lo, cnt_hi]) as usize;
        let entries_start = offset + consts::GROUP_HEADER_LEN;
        let entries_end = entries_start.checked_add(block_length.checked_mul(count)?)?;
        if entries_end > self.frame.len() {
            return None;
        }
        Some(GroupView {
            entries: self.frame.get(entries_start..entries_end)?,
            entry_block_length: block_length,
            count,
        })
    }

    /// Reads a varData field at `offset`: one `u8` length prefix, then
    /// the payload. Returns the payload slice, or `None` when the
    /// length prefix or payload runs past the frame.
    #[must_use]
    pub fn var_data(&self, offset: usize) -> Option<&'a [u8]> {
        let len = *self.frame.get(offset)?;
        let start = offset + consts::VARLEN_PREFIX_LEN;
        let end = start.checked_add(len as usize)?;
        if end > self.frame.len() {
            return None;
        }
        let payload = self.frame.get(start..end)?;
        Some(payload)
    }
}

/// A repeating-group view: sequential fixed-length entries over the
/// group's extent. Entries are accessed by index, bounds-checked.
#[derive(Debug)]
pub struct GroupView<'a> {
    entries: &'a [u8],
    entry_block_length: usize,
    count: usize,
}

impl<'a> GroupView<'a> {
    /// Number of entries the group header declared.
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Bytes per entry.
    #[must_use]
    pub const fn entry_block_length(&self) -> usize {
        self.entry_block_length
    }

    /// The `i`-th entry's bytes, or `None` when `i` is out of range.
    #[must_use]
    pub fn entry(&self, i: usize) -> Option<&'a [u8]> {
        let start = i.checked_mul(self.entry_block_length)?;
        let end = start.checked_add(self.entry_block_length)?;
        let entry = self.entries.get(start..end)?;
        Some(entry)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    /// Builds a frame: the 8-byte header + a body whose bytes are the
    /// body's own offset (so every read has an expected value).
    fn frame_with(body: &[u8]) -> Vec<u8> {
        let mut f = Vec::with_capacity(consts::HEADER_LEN + body.len());
        f.extend_from_slice(&12u16.to_le_bytes()); // block_length
        f.extend_from_slice(&7u16.to_le_bytes()); // template_id
        f.extend_from_slice(&1u16.to_le_bytes()); // schema_id
        f.extend_from_slice(&0u16.to_le_bytes()); // version
        f.extend_from_slice(body);
        f
    }

    #[test]
    fn header_decodes_all_four_fields() {
        let f = frame_with(&[0; 12]);
        let d = SbeDecoder::new(&f);
        let Some(h) = d.message_header() else {
            std::process::abort();
        };
        assert_eq!(h.block_length, 12);
        assert_eq!(h.template_id, 7);
        assert_eq!(h.schema_id, 1);
        assert_eq!(h.version, 0);
    }

    #[test]
    fn short_frame_has_no_header() {
        let d = SbeDecoder::new(&[0u8; 7]);
        assert!(d.message_header().is_none(), "7 bytes < 8-byte header");
    }

    #[test]
    fn scalar_reads_are_bounds_checked() {
        let f = frame_with(&[0; 12]);
        let d = SbeDecoder::new(&f);
        // body starts at 8; a u64 at 8..16 is the last fitting field
        assert!(d.u64_at(8).is_some(), "u64 at 8 fits in a 20-byte frame");
        assert!(d.u64_at(12).is_some(), "u64 at 12 exactly fills 20 bytes");
        assert!(d.u64_at(13).is_none(), "u64 at 13 runs past 20 bytes");
    }

    #[test]
    fn scalar_values_are_little_endian() {
        let mut body = [0u8; 8];
        body[0..8].copy_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        let mut f = frame_with(&body);
        let hdrlen = consts::HEADER_LEN;
        for (i, b) in 0x0102_0304_0506_0708u64.to_le_bytes().iter().enumerate() {
            if let Some(slot) = f.get_mut(hdrlen + i) {
                *slot = *b;
            }
        }
        let d = SbeDecoder::new(&f);
        assert_eq!(
            d.u64_at(hdrlen),
            Some(0x0102_0304_0506_0708),
            "LE byte order preserved"
        );
    }

    #[test]
    fn group_iteration_yields_declared_entries() {
        // A group: blockLength=6, numInGroup=2, then 2 entries of 6 bytes.
        let mut body = Vec::new();
        body.extend_from_slice(&6u16.to_le_bytes()); // entry block length
        body.extend_from_slice(&2u16.to_le_bytes()); // numInGroup
        body.extend_from_slice(b"entry1");
        body.extend_from_slice(b"entry2");
        let mut f = frame_with(&body);
        let hdrlen = consts::HEADER_LEN;
        if let Some(tail) = f.get_mut(hdrlen..) {
            tail.copy_from_slice(&body);
        }
        let d = SbeDecoder::new(&f);
        let Some(g) = d.group_at(hdrlen) else {
            std::process::abort();
        };
        assert_eq!(g.count(), 2);
        assert_eq!(g.entry_block_length(), 6);
        assert_eq!(g.entry(0), Some(&b"entry1"[..]));
        assert_eq!(g.entry(1), Some(&b"entry2"[..]));
        assert_eq!(g.entry(2), None, "index 2 is out of range");
    }

    #[test]
    fn var_data_round_trips() {
        // frame = 8-byte header + [len=5]["hello"] = 14 bytes total.
        let mut f = Vec::new();
        f.extend_from_slice(&[0u8; 8]); // header
        f.extend_from_slice(&[5, b'h', b'e', b'l', b'l', b'o']);
        let d = SbeDecoder::new(&f);
        assert_eq!(d.var_data(8), Some(&b"hello"[..]), "payload at 9..14");
        assert_eq!(d.var_data(9), None, "an inner offset misreads the length");
        assert_eq!(d.var_data(20), None, "past the frame");
    }

    #[test]
    fn group_overflow_is_rejected() {
        // numInGroup declares 4 entries of 6 bytes = 24 bytes, but the
        // frame only carries 12 after the header.
        let mut f = Vec::new();
        f.extend_from_slice(&[0u8; 8]); // header
        f.extend_from_slice(&6u16.to_le_bytes());
        f.extend_from_slice(&4u16.to_le_bytes());
        f.extend_from_slice(&[0u8; 12]);
        let d = SbeDecoder::new(&f);
        assert!(d.group_at(8).is_none(), "declared extent past the frame");
    }
}
