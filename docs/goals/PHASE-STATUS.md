# Phase status — os-core against the corpus (ADR-0009)

- **Status**: point-in-time assessment
- **Date**: 2026-11-21
- **Source of truth**: [index.md](index.md) (keystones, phases, exit
  criteria) + [ADR-0009](../adr/0009-ten-year-goals-phasing.md) (the
  keystones-first strategy). This file is the dated snapshot; the
  index remains canonical.

## Where we are: Phase 1 exit met; standing at the Phase 1 -> 2 boundary

| Phase | Exit criteria (index.md) | Status | Evidence |
|---|---|---|---|
| 0 | spec the vision | **met** (2026-11-09) | ADR-0009 + the corpus split |
| 1 | omni-ring/omni-cap shipped; Lean gate green; Linux bypass spike | **met** (2026-11-20) | 11 published crate-versions (omni-ring 0.2, omni-cap, omni-heap, omni-task, omni-block, omni-sbe 0.1.1, omni-io, omni-tick 0.1/0.2, omni-md, omni-md-server); `make ci` carries the Lean gate (blocking) + Kani; A53 SQPOLL spike numbers |
| 2 | tick-to-trade demo on one board; microcore boots QEMU -> hardware | **in progress** | QEMU boot: done, gate-enforced (blockcheck witness). Hardware boot: not started. Tick-to-trade: the data plane exists (omni-io -> omni-sbe -> omni-md -> omni-tick -> omni-md-server); no end-to-end demo run on a board yet |
| 3 | microcore refinement proof; proof-carrying boot; DSL -> RTL | **rungs landed** | ADR-0014 ladder: mirrors + bitwise bridge + def-form theorems + cursor machine; Kani model-checking. The refinement proof proper (seL4-scale) remains the long pole |
| 4 | integrated heterogeneous stack | not entered | K5/CXL-dependent |

## The six keystones, current standing

| Keystone | Standing | Next artifact |
|---|---|---|
| K1 verified rings + capability algebra | proven + published + model-checked | MAC-sealing handshake doc only if a consumer emerges |
| K2 zero-bypass I/O | QEMU-proven (NVMe full path) + host io_uring seam published | real-metal NVMe smoke; UDP feed socket |
| K3 Lean proof gate | blocking in CI; executable mirrors; Kani pilot | rung D extraction (the decade pole) |
| K4 bare-metal microcore | QEMU boot + NVMe + gated task witness | hardware boot (see the two paths below) |
| K5 FPGA co-synthesis | parked | the board decision |
| K6 language | parked by design (DSL before compiler) | proc-macro DSL after Phase 3 |

## Next goals, ranked (per the corpus, not by preference)

1. **Microcore on real hardware** (Phase 2, K4). Two paths:
   - x86_64 on a dedicated fleet machine (USB/`kexec` boot; ADR-0009
     forbids daily-driver bets) — lowest new-code cost, proves the PVH
     boot + NVMe driver against real silicon;
   - aarch64 port for the A53/S7 — far larger (arch port, UART/GIC
     bring-up), but it is the board the HFT stack already targets.
   Decision needed: which board is "the one board".
2. **Tick-to-trade demo on one board** (Phase 2, hft pillars XXII–XXVI).
   The SOFTWARE form is done (2026-11-21): omni-md's
   `tick_to_trade` example runs the whole plane per tick — SBE
   encode → decode → strategy decide → segment persist — at 33 ns
   min / 36 ns p50 (release), 169 trades over 256 ticks. The BOARD
   form (real feed socket + the A53) is the remaining half.
3. **Server/database first designs** (store/db pillars XLVII–LVI) —
   started: ADR-0017 + omni-tick are pillar LIV's software form
   (521 lock-free tick log; 530 monotonic invariant; 526 epoch/range
   queries this tranche). Queued from LIV: OHLCV
   tick-to-bar synthesis (528). Gorilla XOR compression (522) landed
   2026-11-21 as `omni_tick::gorilla` (software form). LII/LIII (query engines,
   HTAP) queue behind the store. XLIX (NVMe ZNS) queues behind
   real-metal NVMe.
4. **Phase 3 refinement rungs** (K3/K4) — cursor machine formalized;
   next: drive the mirror defs into the kernel's cursor code paths and
   the Kani-vs-Lean cross-check.

## Explicitly parked (per ADR-0009)

- K5 FPGA work: the board decision. Nothing FPGA-shaped starts before
  it.
- K6 language: proc-macro DSL is a Phase-3 artifact.
- Silicon-dependent items: specified or proved as Lean specs only
  (no tape-outs).

## What this file is not

A promise. Exit criteria are met when their evidence survives
`make ci` and (for Phase 2) a hardware run — not when this table says
so.
