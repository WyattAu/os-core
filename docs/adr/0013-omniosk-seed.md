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
3. **Boot path** (updated 2026-11-09, empirically): QEMU `-kernel` for
   64-bit ELFs goes through the **PVH protocol** — the image carries an
   ELF note (type `0x12`, XEN_ELFNOTE_PHYS32_ENTRY) and SeaBIOS hands off
   in 32-bit protected mode with paging possibly still enabled on foreign
   tables. The trampoline therefore: drops PG first, fills a 1 GiB
   identity map (static tables, REX-free 32-bit-safe encodings), sets
   PAE + EFER.LME, re-enables PG into long mode, and lands in the L=1
   code64 descriptor via an **indirect** far jump (m16:32 — the EA-imm
   form is #UD in 64-bit mode). Success/failure is signaled through the
   isa-debug-exit device (guest byte 63 → host exit 127 = success; guest
   33 → 67 = failure; triple-fault reset → 0), asserted by
   `scripts/qemu-omniosk.sh`.
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
