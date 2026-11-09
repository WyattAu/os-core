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
| REQ-OS-001 | `omni-ring`: wait-free bounded SPSC and MPMC rings; loom- and miri-clean; p99 enqueue/dequeue latency benched against `bench/baseline.tsv` | MUST |
| REQ-OS-002 | `omni-cap`: 64-byte capability token type; ring-index and token-algebra theorems machine-checked in Lean 4 (`lake build`, zero `sorry`) | MUST |
| REQ-OS-003 | Lean 4 proof leg in CI: `lake build` (zero `sorry`) over proof crates; advisory first, blocking after one clean loop | MUST |
| REQ-OS-004 | io_uring/DPDK spike validating doorbell-driven submission idioms on Linux; results recorded as a bench note plus a follow-up ADR | SHOULD |

## Traceability Matrix

| Requirement | Check (script, test) | Property class |
|-------------|----------------------|----------------|
| REQ-T-001 | `make ci` == `.github/workflows/ci.yml` step list | structural |
| REQ-T-002 | devcontainer postCreate + `make ci` in CI devcontainer job | integration |
| REQ-OS-001 | loom/miri cfg runs in omni-ring CI; `make bench` vs baseline | model-checked |
| REQ-OS-002 | `lake build` gate over `proofs/` theorem files | machine-checked |
| REQ-OS-003 | CI lean leg (advisory → gate per loop policy) | structural |
| REQ-OS-004 | spike bench note + follow-up ADR | empirical |
