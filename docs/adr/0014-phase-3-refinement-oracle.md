# ADR-14: Phase-3 refinement — oracle-tested correspondence before extraction

- **Status**: Accepted
- **Date**: 2026-11-20

## Context

ADR-0010 states the honest boundary of Phase 1: "the proof-to-Rust link
is structural; a Rust regression cannot (yet) break a Lean build." The
ring theorems (`ring_index_bounded`, `ring_mask_bounded`,
`ring_mask_injective`, `ring_window_injective`,
`ring_inflight_distinct` in `proofs/Proofs.lean`) describe the exact
cursor/mask arithmetic of `omni-ring`'s `SpscRing` — but nothing tied
the two sides together mechanically. Phase 3 (ADR-0009) names
extraction (§48/§100) as the fix; full verified extraction is the
long pole of the decade, not of this tranche.

## Decision

1. **Oracle-tested correspondence lands first.**
   `crates/omni-ring/tests/refinement.rs` executes each ring theorem
   against the real `SpscRing` / `SlotTable`: a model-based FIFO
   refinement proptest over arbitrary push/pop interleavings and
   window slides, a full-window wrap-seam test, and the SlotTable
   payload non-aliasing contract. Every test names the theorem it
   executes. A regression in the cursor/mask/window arithmetic now
   breaks `make ci`.
2. **The link is itself verified load-bearing.** Mutation check
   (2026-11-20): corrupting `try_push`'s slot mask to
   `pos & (CAP - 2)` failed 2 of 3 refinement tests; the reverted
   tree is green. The gate demonstrably fires — "tested" is not a
   label here.
3. **Extraction stays the Phase-3 destination.** The oracle closes
   the regression hole; it does not prove absence of bugs. The Lean
   proofs remain the semantic ground truth, and the eventual
   §48/§100 extraction (Lean-spec-to-executable-code) supersedes
   this oracle rather than deleting it — the proptests become the
   extracted code's conformance suite.

## Consequences

- **Good**: the Phase-1 caveat in ADR-0010 is retired for the ring
  surface: the correspondence is now mechanically enforced in CI,
  not prose.
- **Good**: the pattern is portable — the same
  theorem-named-oracle-tests shape applies to the NVMe CQE theorems
  (`omni-sbe`/kernel), the capability-bound theorem, and the heap
  layout theorems as their surfaces stabilize.
- **Cost**: ~100 lines of proptest in the tier-a gate (sub-second);
  theorem statements and tests must be updated together when the
  arithmetic changes — the doc header in `spsc.rs` points both ways.
- **Risk**: an oracle tests the *sampled* behavior, not the
  universal claim; the theorems remain the only universal
  statements. This is a deliberate hardening rung on the
  verification ladder, not the top rung.
