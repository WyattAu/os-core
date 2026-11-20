//! AVX2 masked-gather ladder (pillar 208, goal 208) — the SIMD
//! counterpart of [`crate::ladder`], compiled only under the `avx2`
//! feature on `x86_64`.
//!
//! Contract: identical to [`crate::ladder::extract_price_ladder`] —
//! same extent validation, same masked-lane-to-zero semantics, and the
//! parity proptest in `tests/ladder.rs` pins the two paths
//! lane-for-lane. The single upfront extent check happens BEFORE any
//! pointer arithmetic, so the gather itself only forms addresses
//! inside the validated frame; masked lanes are architecturally
//! non-faulting (VPGATHERDD masks suppress both the read and any
//! fault on masked elements).
//!
//! Safety contract (ADR-0015): callers MUST confirm AVX2 support at
//! runtime (`is_x86_feature_detected!("avx2")`) or build with
//! `target-feature=+avx2` — this crate is `no_std` and does not
//! detect the CPU itself. The `unsafe` surface is bounded to this
//! module; the portable default remains the workspace posture.
#![cfg(target_arch = "x86_64")]
#![allow(unsafe_code)] // ADR-0015: single-instruction SIMD gather; see module docs

use core::arch::x86_64::{
    _mm256_cmpgt_epi32, _mm256_mask_i32gather_epi32, _mm256_set1_epi32, _mm256_setr_epi32,
    _mm256_setzero_si256, _mm256_storeu_si256,
};

use crate::ladder::validate_extent;

/// AVX2 masked-gather extraction: one `VPGATHERDD` fills the whole
/// ladder. Strides must be a multiple of 4 (SBE aligns u32 fields so)
/// and at most 32 bytes so the byte offsets fit the scale-4 index
/// vector; anything else returns `false` and the caller should use the
/// portable path.
///
/// # Safety
///
/// The CPU must support AVX2 (see the module safety contract). The
/// frame bounds are validated here; masked lanes do not fault.
#[must_use]
#[allow(clippy::cast_ptr_alignment)] // gather elements + storeu are unaligned ops
pub unsafe fn extract_price_ladder_avx2(
    frame: &[u8],
    group_off: usize,
    entry_stride: usize,
    count: usize,
    out: &mut [u32; 8],
) -> bool {
    *out = [0; 8];
    let Some(lanes) = validate_extent(frame, group_off, entry_stride, count) else {
        return false;
    };
    if entry_stride % 4 != 0 || entry_stride / 4 > 8 {
        return false;
    }
    let Ok(l32) = i32::try_from(lanes) else {
        return false;
    };
    let s = i32::try_from(entry_stride / 4).unwrap_or(0);
    // SAFETY: the extent was validated above, so every in-mask lane
    // reads inside `frame`; masked lanes are non-faulting by
    // architecture. AVX2 support is the caller's runtime obligation.
    // (Edition 2021: the `unsafe fn` body is an implicit unsafe block.)
    // The pointer casts claim more alignment than the buffer has, but
    // `vpgatherdd` reads unaligned elements and `_mm256_storeu_si256`
    // stores unaligned by contract — no alignment is ever required.
    let idx = _mm256_setr_epi32(0, s, 2 * s, 3 * s, 4 * s, 5 * s, 6 * s, 7 * s);
    let lane_ids = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
    let mask = _mm256_cmpgt_epi32(_mm256_set1_epi32(l32), lane_ids);
    let base = frame.as_ptr().add(group_off).cast::<i32>();
    let gathered = _mm256_mask_i32gather_epi32::<4>(_mm256_setzero_si256(), base, idx, mask);
    _mm256_storeu_si256(
        out.as_mut_ptr().cast::<core::arch::x86_64::__m256i>(),
        gathered,
    );
    true
}
