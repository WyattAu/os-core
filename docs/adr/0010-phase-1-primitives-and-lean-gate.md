# ADR-10: Phase 1 primitives — rings, capability token, Lean proof gate

- **Status**: Accepted
- **Date**: 2026-11-09

## Context

Phase 0 split the goals corpus and recorded the ten-year phasing
(ADR-0009). Phase 1's exit criteria are the first proven primitives:
`omni-ring` (REQ-OS-001), `omni-cap` (REQ-OS-002), and a Lean 4 proof leg
(REQ-OS-003). Estate survey results constrain the design:

- Estate-best SPSC prior art is vane-kernel's cacheline-padded ring;
  MPMC is delegated to crossbeam everywhere (actor-kit, ferro).
- This workspace's tier-a lint posture denies `unsafe_code` outright, so
  the classic `UnsafeCell` slot protocol is not available here.
- No estate crate ships a 64-byte capability token; no shared CI workflow
  has a Lean leg, and OMNI-CORE 11f (sorry fails, `#print axioms` per
  theorem) existed as policy without an implementation.

## Decision

1. **`omni-ring`**: SPSC is a wait-free bounded ring of `u64` descriptors —
   pure atomics, cache-line-padded cursors, release/acquire handshakes, no
   `unsafe` (safe by construction under the workspace lint contract). The
   `u64`-descriptor scope is deliberate: it is exactly the object the Lean
   theorems talk about (§181 index masking, §163 descriptor tokens), and
   generic-payload SPSC without `unsafe` is tracked as follow-up. MPMC is
   a lock-free wrapper over `crossbeam-queue::ArrayQueue` (estate
   precedent, certified in `supply-chain/audits.toml`). The wait-free vs
   lock-free distinction is stated in the docs per OMNI-CORE 11f.
2. **`omni-cap`**: 64-byte `repr(C, align(64))` token with compile-time
   offset/size assertions (shm-rings `RingHeader` pattern). No `Clone`/`Copy`
   — the Rust shadow of the linear capability algebra. `auth_mac` is an
   opaque reserved field; MAC computation is deferred to a reviewed hashing
   crate rather than hand-rolled.
3. **`proofs/`**: a mathlib-pinned lake project (breaker/proofs skeleton,
   `v4.35.0`) with four theorems: `ring_index_bounded` (§181),
   `ring_mask_bounded` (§162), `perm_pack_bounded` (§184 pattern),
   `capability_contains_bound`. Correspondence to the Rust code is
   structural, not extraction-based (§48 stays a Phase 3 milestone).
4. **Lean gate**: `scripts/check-lean.sh` implements 11f — `lake build`,
   bare-tactic `sorry`/`admit` grep, per-theorem `#print axioms` lines,
   and a `sorryAx` log check. CI runs it as an **advisory** leg
   (`continue-on-error`), graduating to blocking after one clean loop
   (LOOP.md policy, same path perf/repro took). elan pinned 4.0.0 in CI
   (ferro's mitigation for the curl-pipe risk); the nix devshell ships
   elan for local runs.
5. **Bench fix**: `scripts/bench-budget.sh` now resolves the criterion
   directory via `cargo metadata` instead of assuming `./target` — the
   shared-target setups on the fleet redirected it and the baseline pass
   found nothing.
6. **Vet flag**: Makefile `vet` moves to `cargo vet check --locked`
   (cargo-vet 0.10.2 rejects `--all-features` on `check`; the estate
   rust-kit wrapper already invokes `--locked`).

## Consequences

- **Good**: Phase 1 exit criteria are met in one tranche; every artifact is
  gated by the existing tier-a machinery (loom, miri via CI, coverage,
  fuzz, bench) plus the new Lean leg.
- **Good**: the 11f pattern is now executable and portable — the next repo
  with proofs copies `scripts/check-lean.sh`, and a shared `lean-kit.yml`
  becomes a copy-paste (survey confirmed no estate-wide leg exists).
- **Cost**: `make ci` grows a Lean run; first-run mathlib cache download is
  ~9000 files (subsequent runs are incremental).
- **Cost**: generic-payload SPSC and MAC sealing remain open; both are
  REQ-tracked, not silently dropped.
- **Risk**: the proof-to-Rust link is structural; a Rust regression cannot
  (yet) break a Lean build. Extraction (§48/§100) is the Phase 3 fix and
  the honest boundary of what "verified" means here today.
