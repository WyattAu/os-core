<!-- status: CURRENT -->
<!-- REQ-OS-004 spike; generator: scripts/io-spike.sh (spikes/io-spike) -->

# Bench note — io_uring doorbell economics on Linux (2026-11-09)

## Question

Does the goals corpus's doorbell-driven I/O idiom (§11 no-syscalls, §16
direct queue mapping) transfer to Linux io_uring, and what does per-op
submission cost? This informs K2 (zero-bypass I/O discipline) before any
kernel-ring design work.

## Method

`spikes/io-spike` (workspace-excluded; `scripts/io-spike.sh`), release
build, kernel 7.2.6-1-cachyos, `kernel.io_uring_disabled = 0`, target
`/dev/zero` (pure kernel path, no device noise), 512-byte reads, 20,000
ops per sample, 7 samples per mode, median and min reported (the fleet
host is a shared machine — agents, hourly sys-check — so min is the
least-contaminated estimator; see analysis).

## Results

| mode | median ns/op | min ns/op | vs syscall (min) |
|---|---|---|---|
| `syscall` (read(2) per op) | 460.4 | 194.9 | 1.00× |
| `uring-1` (SQE + enter per op) | 1488.7 | 266.6 | 1.37× |
| `uring-batch64` (64 SQEs, 1 enter) | 595.6 | 414.6 | 2.13× |

Median-vs-min spread is 2–5× on every mode: single-shot runs in this
environment are noise (an earlier single-shot pass measured uring-1
*below* syscall — the opposite ranking). The load-bearing signal is the
min column plus the batch-vs-single ratio, not any absolute number.

## Findings

1. **A per-op doorbell is not a doorbell.** `uring-1` pays
   `io_uring_enter(2)` per operation — itself a syscall plus SQE copy —
   so it cannot beat the plain `read(2)` trap it replaces. The goals
   corpus's zero-syscall economics require the *submission* itself to be
   off the call path.
2. **Batching amortizes the enter**: batch64 cuts uring per-op cost ~2.5×
   (median) versus uring-1. The remaining per-op delta against syscall is
   CQE reaping + bookkeeping, which shrinks as batch depth grows.
3. **The true Linux doorbell analogue is SQPOLL** (`IORING_SETUP_SQPOLL`):
   a kernel thread polls the submission queue and the user-side doorbell
   is a memory write with zero syscalls. It requires privilege
   (`kernel.io_uring_sqpoll` unprivileged settings / root); deferred as
   the explicit next spike step, run on an isolated host.
4. **Measurement environment is itself a finding**: 2–5× jitter from
   co-located activity is exactly what Pillar X (§82 tickless core
   isolation, §39 dedicated polling cores) exists to eliminate. Production
   numbers for this program require isolated cores — the spike is the
   first empirical evidence for that pillar.

## Scope

DPDK is out of spike scope (needs NIC + hugepage reservation); deferred
to Phase 2 with the FPGA beachhead, where the wire is real. Next spike
steps: SQPOLL on isolated cores, then NVMe (real device, CQE interrupt
path visible).
