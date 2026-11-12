# ADR-11: Doorbell economics on Linux — batch or SQPOLL, never enter-per-op

- **Status**: Accepted
- **Date**: 2026-11-09

## Context

REQ-OS-004 called for an io_uring/DPDK spike validating doorbell-driven
submission idioms before any kernel-ring design (K2 in ADR-0009). The
spike (`spikes/io-spike`, `scripts/io-spike.sh`, results in
`docs/bench/2026-11-09-io-uring-spike.md`) measured three idioms against
`/dev/zero` on kernel 7.2.6: plain `read(2)` per op, io_uring with one
`io_uring_enter` per op, and io_uring with 64-SQE batches.

## Decision

1. **Never enter-per-op.** io_uring pays for itself only when submission
   is batched (the spike shows enter-per-op ~1.4× *worse* than plain
   `read(2)` at best, batched ~2.5× better than enter-per-op). Any K2
   ring design on Linux submits N descriptors per doorbell.
2. **The Linux doorbell is SQPOLL** (kernel thread polling the SQ;
   user-side submit becomes a memory write). It is privilege-gated, so
   K2 targets it on dedicated/partitioned hosts only, as a follow-up
   spike on isolated cores.
   - **Follow-up (2026-11-12)**: executed on the shared fleet host with
     taskset pinning only — SQPOLL measured 3-4× *worse* than batched
     enter (docs/bench/2026-11-12-sqpoll-spike.md). Index-line bouncing
     between app core and poller thread eats the saved syscall. The
     isolation requirement is now empirical, not assumed.
3. **DPDK is deferred to Phase 2** (FPGA beachhead): it needs a real NIC
   and hugepage reservation; without the wire it measures nothing the
   io_uring numbers didn't already teach.
4. **All performance claims in this program are min-of-N on isolated
   cores** from here on; the spike's 2–5× median-vs-min spread on a
   shared fleet host is recorded as the first empirical argument for
   Pillar X isolation (§82).

## Consequences

- **Good**: the idiom assumption was corrected *before* kernel-ring
  design — enter-per-op would have silently inherited syscall economics.
- **Good**: REQ-OS-004 is satisfied (spike + bench note + this ADR);
  DPDK scope moves to Phase 2 where it belongs.
- **Cost**: follow-up spike work (SQPOLL, NVMe) needs an isolated host;
  the shared fleet machine cannot produce production-grade numbers.
- **Risk**: none carried — the spike is workspace-excluded, unpublished,
  and its crate has no gate obligations.
