//! Pillar-208 ladder tests: the portable extraction is exact against
//! a manual parse, rejects hostile extents, and — under the `avx2`
//! feature on an AVX2 host — agrees lane-for-lane with the masked
//! gather (ADR-0015 parity contract).
//!
//! ADR-0015 unsafe scope: the parity test calls the gather directly;
//! the `unsafe_code` allow below covers exactly that call, under the
//! `avx2` feature only.
#![cfg_attr(feature = "avx2", allow(unsafe_code))]

use omni_sbe::ladder::extract_price_ladder;
use proptest::prelude::*;

/// Little-endian u32 at `off` from a test frame. Callers only ask
/// for validated in-bounds offsets; a bad offset reads as 0.
fn le32(frame: &[u8], off: usize) -> u32 {
    let mut b = [0u8; 4];
    let Some(src) = off.checked_add(4).and_then(|e| frame.get(off..e)) else {
        return 0;
    };
    b.copy_from_slice(src);
    u32::from_le_bytes(b)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Portable path: lanes below the clamped count carry the exact
    /// little-endian levels; masked lanes are zero; a hostile extent
    /// is rejected with the ladder zeroed.
    #[test]
    fn portable_ladder_matches_manual_parse(
        frame in proptest::collection::vec(any::<u8>(), 0..96),
        group_off in 0usize..32,
        stride_pick in 1u32..9,
        count in 0usize..12,
    ) {
        let stride = (stride_pick * 4) as usize;
        let mut out = [0u32; 8];
        let ok = extract_price_ladder(&frame, group_off, stride, count, &mut out);
        let lanes = count.min(8);
        let need = stride.checked_mul(lanes);
        let fits = matches!(need, Some(n) if group_off + n <= frame.len());
        prop_assert_eq!(ok, fits, "acceptance matches the declared extent");
        if !fits {
            prop_assert_eq!(out, [0u32; 8], "rejected ladder is zeroed");
            return Ok(());
        }
        for (lane, v) in out.iter().enumerate() {
            if lane < lanes {
                prop_assert_eq!(*v, le32(&frame, group_off + lane * stride), "lane {}", lane);
            } else {
                prop_assert_eq!(*v, 0, "masked lane {} is zero", lane);
            }
        }
    }

    /// AVX2 masked-gather parity (ADR-0015): on an AVX2 host, the SIMD
    /// path must agree lane-for-lane with the portable path across the
    /// whole random case space, including rejections. Skips silently
    /// on hosts without AVX2 — CI runners and the dev fleet have it.
    #[test]
    #[cfg(all(target_arch = "x86_64", feature = "avx2"))]
    fn avx2_parity_with_portable(
        frame in proptest::collection::vec(any::<u8>(), 0..96),
        group_off in 0usize..32,
        stride_pick in 1u32..9,
        count in 0usize..12,
    ) {
        if !std::is_x86_feature_detected!("avx2") {
            return Ok(());
        }
        let stride = (stride_pick * 4) as usize;
        let mut portable = [0u32; 8];
        let mut simd = [0u32; 8];
        let ok_p = extract_price_ladder(&frame, group_off, stride, count, &mut portable);
        let ok_s = unsafe {
            // SAFETY: AVX2 support confirmed by is_x86_feature_detected above.
            omni_sbe::simd::extract_price_ladder_avx2(
                &frame,
                group_off,
                stride,
                count,
                &mut simd,
            )
        };
        prop_assert_eq!(ok_p, ok_s, "acceptance agrees");
        prop_assert_eq!(portable, simd, "lanes agree");
    }
}

/// Stride smaller than a lane is rejected, not wrapped.
#[test]
fn tiny_stride_rejected() {
    let frame = [0xABu8; 16];
    let mut out = [0u32; 8];
    assert!(
        !extract_price_ladder(&frame, 0, 3, 2, &mut out),
        "stride 3 < 4"
    );
    assert_eq!(out, [0u32; 8]);
}

/// Count above 8 clamps: exactly 8 lanes come out, extras ignored.
#[test]
fn count_clamps_to_lanes() {
    // Lane i carries the little-endian value i + 1 (first byte set,
    // rest zero).
    let mut frame = [0u8; 8 * 4];
    for (lane, b) in frame.chunks_exact_mut(4).enumerate() {
        let v = u8::try_from(lane + 1).unwrap_or(0);
        b.copy_from_slice(&[v, 0, 0, 0]);
    }
    let mut out = [0u32; 8];
    assert!(extract_price_ladder(&frame, 0, 4, 20, &mut out));
    for (i, v) in out.iter().enumerate() {
        let want = u32::try_from(i + 1).unwrap_or(0);
        assert_eq!(*v, want, "lane {i}");
    }
}
