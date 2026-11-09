# ADR-13: omniosk seed — library-first, QEMU `-kernel` path, one unsafe boundary

- **Status**: Accepted
- **Date**: 2026-11-09

## Context

Phase 2 entry (ADR-0009): the bare-metal seed (K4). The workspace denies
`unsafe_code` outright — correct for userland crates, incompatible with a
kernel that must touch UART ports, page tables, and MSRs. Goals targets:
§51 firmware-free boot, §200 naked entry, §35 Ring-0 flat unikernel,
§191 tickless cores, with omni-ring's Lean-proved descriptors as the
kernel's first transport (§161–§163).

## Decision

1. **Library-first.** `crates/omniosk` lands as a `no_std` library whose
   logic is host-testable behind seams: `serial::PortIo` abstracts port
   I/O (byte-exact init/write sequences tested against a mock), and
   `qemu_exit` encodes the isa-debug-exit contract. Nothing unsafe is in
   the workspace yet; the metal implementation of `PortIo` arrives with
   the kernel binary.
2. **One unsafe boundary, audited.** The kernel bin (next step) overrides
   `unsafe_code` for itself with ADR-backed justification — the deviation
   is scoped to the bin, never the estate crates. This is the sanctioned
   mechanism the deny-all lint needs to coexist with an OS program.
3. **Boot path**: multiboot1 header → QEMU `-kernel` (no GRUB/stage-2 in
   the seed) → 32-bit entry sets up identity page tables and EFER.LME,
   jumps to 64-bit `kernel_main` (§200 naked-entry discipline: `xor
   rbp`, set `rsp`, no CRT) → COM1 console (this crate) → run the
   omni-ring SPSC smoke (verified descriptors, bare metal, §161) →
   tickless idle loop (§391 `hlt`) → `isa-debug-exit` success (§33/35).
4. **Gate strategy**: omniosk *library* code is tier-a (clippy, tests,
   coverage) like every estate crate; the kernel *bin* is a process
   surface excluded from the coverage gate via the ADR-0005 regex
   (extended when the bin lands), and its correctness argument is the
   QEMU smoke leg asserting serial output and exit status.
5. **Harness**: `scripts/qemu-omniosk.sh` builds the bin for
   `x86_64-unknown-none`, runs QEMU headless with `-serial stdio -device
   isa-debug-exit`, asserts the PASS marker in serial output and exit
   status. CI gets a QEMU smoke job when the bin exists.

## Consequences

- **Good**: Phase 2 starts inside the estate gates instead of outside
  them; every kernel-adjacent byte that can be tested on the host is.
- **Good**: the unsafe boundary is one audited place, not a gradual
  erosion of the deny posture.
- **Cost**: the seed is a library plus decided path — the machine does
  not boot yet; next session lands boot asm, the bin, and the QEMU leg.
- **Risk**: multiboot1 via `-kernel` caps the seed at what QEMU's loader
  supports; if Stage-2 needs more (framebuffer, ACPI), ADR-0014 revisits
  the boot contract.
