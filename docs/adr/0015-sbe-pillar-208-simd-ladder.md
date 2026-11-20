# ADR-15: omni-sbe pillar-208 ladder — portable default, feature-gated AVX2 gather

- **Status**: Accepted
- **Date**: 2026-11-20

## Context

Goal 208 (wire-speed ingestion pillar): branchless SBE price-ladder
extraction via single-instruction gather. omni-sbe shipped the
portable baseline (v0.1.0) with a "zero unsafe" posture — correct for
an estate crate ingesting untrusted wire bytes. The SIMD gather
(`VPGATHERDD`) is an `unsafe` intrinsic, so the full form cannot land
under the workspace's blanket `unsafe_code = deny` without a scoped,
ADR-backed deviation (the mechanism ADR-0013 sanctioned for the
kernel bin).

## Decision

1. **Portable stays the default and the contract.**
   `ladder::extract_price_ladder` remains safe, `no_std`,
   branch-free-per-lane (single upfront extent check, mask-multiply
   lane fill), and is what every consumer gets without opt-in.
2. **The SIMD path is a compile-time opt-in, not a runtime default.**
   `simd::extract_price_ladder_avx2` lives behind
   `feature = "avx2"` on x86_64 only. The crate does no CPU
   detection; callers detect (`is_x86_feature_detected!`) or build
   with `target-feature=+avx2`. This keeps the `no_std` profile and
   puts the unsafe obligation on the caller, explicitly.
3. **The unsafe surface is exactly one module, one function, and its
   parity test.**
   `#![allow(unsafe_code)]` sits on `simd.rs` with the ADR reference
   in its safety contract; the parity test file carries a
   feature-scoped `cfg_attr` allow for its single direct call. The
   module documents why the gather is
   safe-by-construction: the extent is validated before any pointer
   arithmetic, and masked lanes of `VPGATHERDD` are architecturally
   non-faulting. Any further unsafe site needs its own ADR.
4. **Parity is machine-checked, not asserted in prose.**
   `tests/ladder.rs` runs a proptest comparing the AVX2 path
   lane-for-lane with the portable path over random frames, offsets,
   strides, and counts — including rejections — on every AVX2-capable
   host that runs `--all-features` (the CI gate shape).

## Consequences

- **Good**: pillar 208's full form exists end-to-end (portable
  baseline + single-instruction extraction) without weakening the
  estate's zero-unsafe posture anywhere else.
- **Good**: the deviation is reviewable in one place; `git grep
  allow(unsafe_code)` in the estate returns exactly the kernel bin
  and this module.
- **Cost**: `--all-features` CI now compiles the x86_64 intrinsics
  (~2 KB object code, no default-path impact).
- **Risk**: the parity proptest only runs on AVX2-capable hosts;
  non-AVX2 CI environments silently skip it. The dev fleet and
  current GitHub runners are AVX2-capable — if a runner class loses
  that, the skip must become a hard fail.

## Notes

- The pillar text says "AVX-512" but its sample instruction is the
  AVX2 256-bit masked gather (`_mm256_mask_i32gather_epi32` is the
  `vpgatherdd` ymm form). We implement the sample's width (8 lanes);
  an AVX-512 16-lane variant is a straightforward extension behind
  the same ADR when a consumer needs it.
