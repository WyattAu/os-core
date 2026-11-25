# ADR-18: tick store v2 — pillar LIV software forms, sequenced

- **Status**: Accepted
- **Date**: 2026-11-21

## Context

The server/database first designs (the user's Phase-2 directive) map
onto the corpus's `store`/`db` pillars XLVII–LVI. Pillar LIV
(Ultra-Fast Time-Series & Financial Tick Data Engines, goals 521–530)
is the direct continuation of omni-tick: the segment store is goal
521's software form (lock-free append, zero WAL, zero serialization),
and two of LIV's items are actionable in pure software right now
before any FPGA/GPU form exists.

## Decision

1. **530 first — the monotonic invariant, software form.**
   `SegmentBuf::push_monotonic` enforces strictly advancing `ts_ns`
   (duplicates and out-of-order rejected, never reordered — the
   tie-break injection of the pillar's hardware form is an ingress
   concern, not a store concern). The mirror def
   (`store_last_ts`) and its monotonicity theorem are in
   `proofs/Proofs.lean`: the store's ordering anchor provably never
   moves backward.
2. **526 second — epoch/range queries, pointer-arithmetic form.**
   `SegmentBuf::range(ts_start, ts_end)` binary-searches the
   monotonic record order (partition point) and yields the half-open
   slice. Specified for monotonic segments only; a non-monotonic
   segment's range output is documented as unspecified, not undefined.
3. **Queued from LIV, in order**: 522 Gorilla XOR compression
   (segment-level storage shrink — the largest software win left),
   528 OHLCV tick-to-bar synthesis (an aggregator over the replay
   stream), 521's hardware-counter timestamp tagging (ingress-side,
   A53 session). The FPGA/GPU forms (523/524/525/527/529) are K5-
   gated per ADR-0009.
4. **Query surfaces (LII/LIII) queue behind the store.** omni-md's
   HTTP endpoints are the v0 query surface; a relational/algebraic
   query layer (LII 501–510) waits until the store's invariants and
   format stabilize at 0.3+.

## Consequences

- **Good**: omni-tick 0.2.1 carries LIV's two software-actionable
  items with Lean + Kani coverage; the pillar items become a
  checklist, not a wishlist.
- **Good**: the monotonic contract is opt-in (`push` stays
  format-agnostic), so non-monotonic feeds keep a documented escape
  hatch.
- **Cost**: the monotonic check is one comparison per push — inside
  the budget; the range query's binary search is O(log n) per query.
- **Risk**: `range` on a non-monotonic segment is *unspecified*
  output — documented, but a caller could still do it; a debug-mode
  monotonic scan is a possible future guard.

**Update 2026-11-21 (522 landed):** Gorilla XOR compression shipped as
`omni_tick::gorilla` — the software form of goal 522. Format: first
value raw LE, then per value a control bit (unchanged = one bit) or
6-bit trailing zeros + 6-bit (length - 1) + payload. Trailing+length
(derived leading) rather than Gorilla's leading+length: a 5-bit lead
field cannot express deep-lead deltas, which is exactly where small
price drift lives. Roundtrip proptest over arbitrary streams; drift
streams compress below half raw. The FPGA single-cycle pipeline form
(522's full claim) stays K5-gated; 528 (OHLCV bars) is the next
software item in the LIV queue.

**Update 2026-11-22 (528 landed):** `omni_tick::bars` — the OHLCV
tick-to-bar synthesizer, goal 528's software form. Fixed-epoch
buckets, the completed bar emitted at the epoch crossing, empty
epochs emit nothing (facts, not interpolations), `flush` for
end-of-stream. Reference price = `bid_px` (quote-tick convention,
consistent with omni-md's strategy). The hardware DSP-pipeline form
(528's full claim) stays K5-gated; the LIV software queue is now
521-530 minus the FPGA/GPU forms.
