//! Portable branchless price-ladder extraction (pillar 208 baseline,
//! REQ layer: goal 208 of the wire-speed ingestion pillar).
//!
//! Extracts up to 8 little-endian `u32` price levels from a repeating
//! group's fixed-stride entries. "Branchless" in the pillar's sense:
//! the extent is validated ONCE up front, and every lane is filled
//! with a mask multiply — no per-element data-dependent branches, no
//! per-element bounds panics. This is the default path; the AVX2
//! masked gather (`crate::simd`, feature `avx2`) implements the same
//! contract in one instruction and must agree lane-for-lane.

/// Lane count of the ladder (the pillar's 8-wide vector).
pub(crate) const LANES: usize = 8;

/// Validates the ladder extent once: the group offset, stride, and
/// count must place every in-mask lane fully inside the frame.
/// Returns the clamped lane count, or `None` when the extent is
/// hostile. One check per ladder, not per element.
pub(crate) fn validate_extent(
    frame: &[u8],
    group_off: usize,
    entry_stride: usize,
    count: usize,
) -> Option<usize> {
    if entry_stride < 4 {
        return None; // a u32 lane cannot fit in a smaller stride
    }
    let lanes = count.min(LANES);
    let need = entry_stride.checked_mul(lanes)?;
    let end = group_off.checked_add(need)?;
    if end > frame.len() {
        return None;
    }
    Some(lanes)
}

/// Extracts up to 8 little-endian `u32` price levels starting at
/// `group_off`, entries spaced `entry_stride` bytes apart, taking
/// `count` entries (clamped to 8).
///
/// Lanes at or beyond the clamped count are written as `0` (the
/// pillar's masked-out lanes). Returns `false` — leaving `out`
/// all-zero — when the extent does not fit the frame, the stride is
/// smaller than a lane, or the arithmetic overflows: untrusted wire
/// input is rejected, never wrapped.
#[must_use]
pub fn extract_price_ladder(
    frame: &[u8],
    group_off: usize,
    entry_stride: usize,
    count: usize,
    out: &mut [u32; 8],
) -> bool {
    *out = [0; 8];
    let Some(_lanes) = validate_extent(frame, group_off, entry_stride, count) else {
        return false;
    };
    for (lane, slot) in out.iter_mut().enumerate() {
        let take = u32::from(lane < count);
        let o = lane
            .checked_mul(entry_stride)
            .and_then(|d| group_off.checked_add(d))
            .unwrap_or(0);
        let v = o
            .checked_add(4)
            .and_then(|e| frame.get(o..e))
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map_or(0, u32::from_le_bytes);
        *slot = v * take;
    }
    true
}
