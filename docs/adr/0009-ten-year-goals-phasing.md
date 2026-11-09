# ADR-9: Phase the goals corpus — keystones first, FPGA beachhead, no tape-outs

- **Status**: Accepted
- **Date**: 2026-11-09

## Context

`goals.md` (now [`docs/goals/`](../goals/index.md)) describes a formally
verified, zero-bypass, heterogeneous-computing operating system in 1,250
numbered breakthroughs across 126 pillars: SASOS with cryptographic
capabilities, doorbell-driven I/O with no syscalls, CPU/GPU/FPGA as
scheduling peers, Lean 4 proofs on every fast path, a custom systems
language compiling to machine code and RTL, and application volumes (HFT
silicon, security by construction, driverless rendering/ML,
storage-as-database, quantum control, and more).

As a single program this is unsplashable: it stacks seL4-scale verification,
CHERI-scale capability silicon, and a full FPGA trading stack. Attempting it
in corpus order guarantees effort spent on items whose dependencies do not
exist. Three environment facts change the economics, and the roadmap uses
all three:

1. AI-assisted proof search compresses Lean verification work (the estate
   already runs lean-lsp for proof feedback).
2. FPGAs are reprogrammable silicon simulators: ideas that "need custom
   silicon" can be evidenced on a board in weeks.
3. CXL memory pooling becomes commodity hardware inside the roadmap window.

## Decision

**Keystones first; HFT is the revenue engine; silicon is FPGA now, IP later.**

1. Six keystones order all work (map in `docs/goals/index.md`):
   K1 verified rings + capability algebra, K2 zero-bypass I/O, K3 Lean proof
   gate, K4 bare-metal microcore, K5 FPGA co-synthesis, K6 custom language
   (DSL before compiler).
2. Ten-year phases (exit criteria in the index): 0 spec the vision (this
   ADR + the corpus split); 1 proven userspace primitives; 2 FPGA beachhead
   + bare-metal seed; 3 verified microkernel + language v1; 4 integrated
   heterogeneous stack.
3. The HFT pillars (XXII–XXVI) are built as a product, not research: they
   consume K1/K2/K5, fund hardware, and force the low-latency disciplines.
4. Linux remains the management plane through Phase 4; the research kernel
   runs on dedicated partitions/boards, never as a daily-driver bet.
5. No tape-outs. Items requiring new transistors (CHERI-style tags, PUF,
   ReRAM, photonics) are specified, proved as Lean specifications, targeted
   at FPGA, or licensed as IP.
6. Physics/category pillars (XII–XVI) are modeling lenses, never milestones.

## Consequences

- **Good**: every phase produces shippable artifacts (crates, proofs, FPGA
  demos) — no decade-long dark tunnel; revenue starts in Phase 2.
- **Good**: the corpus becomes navigable; items acquire a phase only via a
  keystone or application tag.
- **Cost**: anything not keystone- or application-tagged is explicitly
  deferred; contributors may find a favorite pillar parked.
- **Cost**: the microcore refinement proof (Phase 3) remains the long pole;
  it starts only after K1/K2 artifacts exist to prove against.
- **Risk**: single-person dependency across a decade — mitigated by plan
  ledgers, the agent fleet, and this repo's template contract.
