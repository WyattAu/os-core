# Requirements — OmniRust-template

Numbered, testable requirements. Every requirement maps to at least one named
test/script check; every security-relevant check cites at least one
requirement. Doc comments on the implementing item carry `REQ-NNN` tags.

## Template (toolchain) requirements

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-T-001 | `make ci` runs the same scripts CI runs; no CI-only steps | MUST |
| REQ-T-002 | Fresh clone → green `make ci` inside the devcontainer with zero manual steps | MUST |
| REQ-T-003 | Both devcontainer flavors (image, nix) build and reach green CI | MUST |
| REQ-T-004 | Monorepo layout demonstrates one internal dependency between packages | MUST |

## Security

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-T-100 | No secrets in the scaffold; `.env.example` pattern only | MUST |
| REQ-T-101 | CI workflows run with least-privilege `permissions:` blocks | MUST |

## OS program (Phase 0→1 tranche)

The vision corpus lives in [`docs/goals/`](docs/goals/index.md); phasing in
[ADR-0009](docs/adr/0009-ten-year-goals-phasing.md). This tranche covers the
first shippable artifacts only.

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-OS-001 | `omni-ring`: wait-free bounded SPSC ring (`u64` descriptors, generic payload tracked as follow-up) and lock-free bounded MPMC ring (`crossbeam-queue`, estate precedent); loom- and miri-clean; throughput benched against `bench/baseline.tsv` | MUST |
| REQ-OS-002 | `omni-cap`: 64-byte cache-line-aligned capability token (bounds, permissions, epoch; `auth_mac` sealed via domain-separated SipHash-2-4 lanes, ADR-0012); ring-index and token-algebra theorems machine-checked in Lean 4 (`lake build`, zero `sorry`) | MUST |
| REQ-OS-003 | Lean 4 proof leg in CI: `lake build` (zero `sorry`) over `proofs/`; advisory first, blocking after one clean loop | MUST |
| REQ-OS-004 | io_uring/DPDK spike validating doorbell-driven submission idioms on Linux; results recorded as a bench note plus a follow-up ADR | SHOULD |
| REQ-OS-005 | `omniosk` seed (K4): boot-capable `no_std` library modules (serial `PortIo` seam, qemu-exit, bare-metal ring integration) host-tested under tier-a gates; kernel bin + QEMU harness per ADR-0013 | SHOULD |
| REQ-OS-006 | NVMe block-write path in `omniosk`: WRITE one block to LBA 1 over the I/O queues, read it back, byte-compare in-kernel; the full block path (read + write + verify) asserted in the QEMU smoke gate | SHOULD |
| REQ-OS-007 | `omni-heap`: deterministic bitmap page allocator for the microcore — borrowed bitmap (kernel `static` or stack array), `no_std`, panic-free (every failure is a `Result`), first-fit deterministic layout | SHOULD |
| REQ-OS-008 | Microcore tick + sleep: PIT-backed ~1 kHz tick (PIC remapped to 0x50+, IRQ0 only), IDT with 64-bit gates + fault gates, interrupt-driven `hlt` sleep with a no-progress guard; `omni-task` cooperative round-robin task slots consume the tick | SHOULD |

## Traceability Matrix

| Requirement | Check (script, test) | Property class |
|-------------|----------------------|----------------|
| REQ-T-001 | `make ci` == `.github/workflows/ci.yml` step list | structural |
| REQ-T-002 | devcontainer postCreate + `make ci` in CI devcontainer job | integration |
| REQ-OS-001 | loom/miri cfg runs in omni-ring CI; `make bench` vs baseline | model-checked |
| REQ-OS-002 | `lake build` gate over `proofs/` theorem files | machine-checked |
| REQ-OS-003 | CI lean leg (advisory → gate per loop policy) | structural |
| REQ-OS-004 | spike bench note + follow-up ADR | empirical |
| REQ-OS-005 | omniosk unit tests over mock port sink; boot path per ADR-0013 | unit + design |
| REQ-OS-006 | `NVME WRITE PASS` serial marker, asserted required in `scripts/qemu-omniosk.sh` (write -> read-back -> byte-compare in-kernel) | empirical |
| REQ-OS-007 | proptest model equivalence (`crates/omni-heap/tests/model.rs`) + `heap_page_aligned` / `heap_page_in_range` Lean theorems | model-checked |
| REQ-OS-008 | `TIMER PASS` + `SLEEP PASS` serial markers asserted required in `scripts/qemu-omniosk.sh` (tick presence + 250 ms calibration within tolerance); `crates/omni-task` fairness proptests (exact rotation, no starvation under churn) | empirical + model |
