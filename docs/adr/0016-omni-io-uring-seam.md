# ADR-16: omni-io — raw-syscall io_uring seam (K2's Linux surface)

- **Status**: Accepted
- **Date**: 2026-11-20

## Context

K2 (zero-bypass I/O) has two proven halves: the QEMU in-kernel NVMe
driver and the A53 io_uring spike (REQ-OS-004: SQPOLL batch 5,129 ns/op
vs enter 8,105). The seam that turns the spike into a product — a
published crate the HFT pillars consume — was missing. The obvious
alternative, a third-party `io-uring` binding, trades one vet for a
version-skew surface: ring layouts, opcode availability, and feature
flags move per kernel release, and the binding's abstraction becomes
our abstraction.

## Decision

1. **Own the wire format.** omni-io speaks `io_uring_setup` /
   `io_uring_enter` / `mmap` directly over libc. Ring layouts are read
   back from the device (`io_uring_params` offsets — never hardcoded),
   so 5.10+ kernels work unchanged. This is the omni-sbe posture
   (own-the-bytes) applied to K2.
2. **`std`-only, Linux-only — deliberately.** The rings are mmap'd
   shared pages; a `no_std` story needs a hero allocator and buys
   nothing (ADR-0003's std-gating applies: OS-bound leaves are gated,
   and this leaf is nothing but OS).
3. **The unsafe surface is the whole crate, scoped by this ADR.** mmap
   rings are raw-pointer construction by nature (the kernel hands back
   shared pages). Every block carries a SAFETY comment; the
   cursor/handle arithmetic is the K1 algebra of record
   (`spsc_slot_bounded` / `spsc_slot_injective` — `tail & mask` cannot
   leave the ring), and the proptests of the rings' surface stay in
   omni-ring. The estate's `unsafe_code` deviations are now: the
   kernel bin (ADR-0013), the omni-sbe SIMD module (ADR-0015), and
   this crate (ADR-0016) — each with its own record.
4. **v0.1 scope: setup/teardown, `NOP`, `READ`, `WRITE`,
   submit-and-wait, drain.** SQPOLL (batched, per the A53 numbers),
   registered buffers, and multi-submitter hardening are the next
   tranche. Host smokes prove the doorbell path end-to-end (NOP drain,
   file roundtrip) and skip cleanly on io_uring-less hosts.

## Consequences

- **Good**: the K2 seam is a published-crate candidate immediately;
  the HFT pillars compose it with omni-ring + omni-sbe + omni-task.
- **Good**: kernel-version exposure is one struct pair and three
  syscall numbers, all read-back-validated.
- **Cost**: raw-syscall ownership means QEMU-style quirks (doorbell
  semantics) are ours to find — the kernel session's cursor-wrap bug
  is the reminder; the smokes + the A53 hardware session are the
  mitigation.
- **Risk**: single-submitter v0.1 is not multi-core-safe yet; the
  multi-submitter tranche must land before any actor claims it (K2's
  kernel-side consumption stays in-kernel until then).
