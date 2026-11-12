<!-- status: CURRENT -->

# SQPOLL spike — the doorbell as a memory write (ADR-0011 follow-up)

**Date**: 2026-11-12 · **Host**: shared 6-core fleet machine (Arch, kernel 7.2.6)
**Method**: taskset -c 4,5 (app + SQPOLL kernel thread), min-of-7, /dev/zero, 512B reads, 20k ops
**Verdict**: on a shared host, SQPOLL does **not** beat a batched enter —
index-line bouncing between the app core and the kernel thread eats the
saved syscall. This *confirms* ADR-0011's "dedicated/partitioned hosts
only" qualifier with data, and sharpens K2's design constraint: the
memory-write doorbell pays off only with CPU isolation.

## Results (min ns/op; median in parens)

| mode | ns/op (min) | ns/op (median) | vs syscall |
|---|---:|---:|---:|
| syscall (read(2) per op) | 113.2 | 166.5 | 1.0× |
| uring-1 (enter per op) | 346.2 | 375.7 | 3.1× worse |
| uring-batch64 | 131.5 | 201.4 | 1.2× worse |
| sqpoll-1 | 735.6 | 1252.4 | 6.5× worse |
| sqpoll-batch64 | 427.3 | 1007.2 | 3.8× worse |

Run: `taskset -c 4,5 scripts/io-spike.sh 20000`; TSV rows reproduced in
`docs/bench/` history.

## Findings

1. **SQPOLL lost to batched-enter by ~3× (min) on this host.** The
   kernel SQ-poller thread does not inherit the task's affinity; every
   SQ tail / CQ head update bounces cache lines between the app core
   and the poller's core. One cross-core round trip costs more than the
   syscall it replaces. (ADR-0011 decision 2 said SQPOLL targets
   "dedicated/partitioned hosts"; this is the measured reason.)
2. **Per-op SQPOLL was the worst mode** (735 ns min): a memory-write
   doorbell per op still costs the kernel thread's wake/queue walk per
   op — SQPOLL amortizes the *syscall*, not the per-op work.
3. **Batching remains the only reliable win**, as in the 2026-11-09
   spike: uring-batch64 stays within 1.2× of syscall while removing the
   trap from the data path.
4. **Methodology caveat**: the SQPOLL modes spin-reap completions
   (documented in `spikes/io-spike/src/main.rs`), so the app thread and
   the poller compete for the pinned pair. A blocking-wait variant
   would trade that for a syscall per batch — no configuration on a
   shared host gives SQPOLL a fair stage.

## Decision impact

- **Unchanged**: ADR-0011's rules (batch, never enter-per-op) hold.
- **Sharpened**: K2's memory-write doorbell design MUST be evaluated on
  an isolated-CPU host (isolcpus/nohz_full + IRQ affinity) before any
  performance claim; taskset pinning is insufficient.
- **New empirical datum for Pillar X (§82)**: shared-host noise flipped
  a theoretically-winning idiom into a measured loss — the second
  recorded instance after the 2–5× median-vs-min spread.
