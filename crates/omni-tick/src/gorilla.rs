//! Gorilla-style XOR compression for u64 price streams (pillar LIV
//! goal 522's software form): adjacent values are `XORed` with the
//! previous value; an unchanged value costs one bit, a changed one
//! costs a control bit plus its meaningful payload. On realistic
//! price drift (small deltas, shared high bits) this shrinks a u64
//! stream by ~80% — the pillar's number.
//!
//! Format (per value after the first, which is stored raw LE):
//! - control bit `0` → value == previous
//! - control bit `1`, 6-bit trailing zeros, 6-bit (length - 1), then
//!   the meaningful payload → changed value
//!
//! Trailing zeros + length (Gorilla's own layout) rather than leading
//! zeros: the leading count is derivable (64 - trail - length) and a
//! 5-bit leading field cannot express the deep-lead values that make
//! small deltas compress.
//!
//! Zero allocation, zero unsafe, zero panics: the writer/reader are
//! bounds-checked and report exhaustion, never crash.

/// A bit-level writer over a caller-owned byte buffer.
struct BitWriter<'a> {
    buf: &'a mut [u8],
    /// Next bit position to write.
    pos: usize,
}

impl<'a> BitWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Appends `count` bits from `value` (MSB-first). `false` when
    /// the buffer is exhausted.
    fn write_bits(&mut self, value: u64, count: u32) -> bool {
        for i in (0..count).rev() {
            let bit = (value >> i) & 1;
            let byte = self.pos / 8;
            let Some(cell) = self.buf.get_mut(byte) else {
                return false;
            };
            let mask = 1 << (7 - self.pos % 8);
            if bit == 1 {
                *cell |= mask;
            } else {
                *cell &= !mask;
            }
            self.pos += 1;
        }
        true
    }

    /// The byte length the written bits occupy.
    fn written_bytes(&self) -> usize {
        self.pos.div_ceil(8)
    }
}

/// A bit-level reader over a compressed buffer.
struct BitReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Reads `count` bits (MSB-first); `None` past the buffer.
    fn read_bits(&mut self, count: u32) -> Option<u64> {
        let mut value = 0u64;
        for _ in 0..count {
            let byte = self.buf.get(self.pos / 8)?;
            let bit = (byte >> (7 - self.pos % 8)) & 1;
            value = (value << 1) | u64::from(bit);
            self.pos += 1;
        }
        Some(value)
    }
}

/// Compresses `values` into `out`, returning the compressed byte
/// length. `None` when `out` is too small (the caller grows or rolls
/// the block — the format has no partial states).
pub fn gorilla_encode(values: &[u64], out: &mut [u8]) -> Option<usize> {
    let mut w = BitWriter::new(out);
    // Header: the value count (u32 LE) then the first value raw.
    let count = u32::try_from(values.len()).ok()?;
    if !w.write_bits(u64::from(count), 32) {
        return None;
    }
    let Some(&first) = values.first() else {
        return Some(w.written_bytes()); // count-0 stream: header only
    };
    let mut prev: u64 = first;
    if !w.write_bits(prev, 64) {
        return None;
    }
    for &v in values.iter().skip(1) {
        let xor = v ^ prev;
        if xor == 0 {
            if !w.write_bits(0, 1) {
                return None;
            }
            prev = v;
            continue;
        }
        let trail = xor.trailing_zeros();
        let width = 64u32 - trail - xor.leading_zeros(); // nonzero xor: >= 1
        if !w.write_bits(1, 1)
            || !w.write_bits(u64::from(trail), 6)
            || !w.write_bits(u64::from(width - 1), 6)
            || !w.write_bits(xor >> trail, width)
        {
            return None;
        }
        prev = v;
    }
    Some(w.written_bytes())
}

/// Decompresses `count` values written by [`gorilla_encode`] back
/// into `out`. `None` on any malformed bitstream (truncated payload,
/// impossible control words).
///
/// # Errors
///
/// None — malformed input yields `None`, never a panic.
#[must_use]
pub fn gorilla_decode(compressed: &[u8], count: usize, out: &mut [u64]) -> Option<()> {
    if out.len() < count {
        return None;
    }
    let mut r = BitReader::new(compressed);
    let stored_count = r.read_bits(32)?;
    let stored_count = usize::try_from(stored_count).ok()?;
    if count == 0 {
        return Some(());
    }
    if stored_count < count {
        return None;
    }
    let mut prev = r.read_bits(64)?;
    let mut slots = out.iter_mut();
    let first = slots.next()?;
    *first = prev;
    for slot in slots {
        if r.read_bits(1)? == 0 {
            *slot = prev;
            continue;
        }
        let trail = u32::try_from(r.read_bits(6)?).ok()?;
        let width = u32::try_from(r.read_bits(6)?).ok()?.checked_add(1)?;
        let payload = r.read_bits(width)?;
        // Validity: trail + width <= 64 (the encode side guarantees
        // it; a malformed stream with trail + width > 64 reads past
        // the buffer and fails above).
        let _ = 64u32.checked_sub(trail)?.checked_sub(width)?;
        *slot = prev ^ (payload << trail);
        prev = *slot;
    }
    Some(())
}
