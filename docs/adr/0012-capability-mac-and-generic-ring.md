# ADR-12: Capability MAC sealing and the generic-ring decision

- **Status**: Accepted
- **Date**: 2026-11-09

## Context

ADR-0010 left two REQ-tracked follow-ups open: `omni-cap`'s `auth_mac`
was a reserved field with sealing deferred to a reviewed crate, and
generic-payload SPSC transport was parked until a safe owned-slot design
existed. Both block Phase 1.5 (publication), so both needed a decision,
not just a deferral.

## Decision

1. **Sealing**: `omni-cap::SealKey` (16-byte SipHash key) and
   `HardwareCapability::{seal, verify}`. The MAC is four
   domain-separated SipHash-2-4 lanes over the canonical little-endian
   field bytes (base, bound, permissions, pasid, epoch — 32 bytes; the
   lane index prefixes the hashed content). Verification is
   constant-time (xor-fold, no early exit). SipHash-2-4 comes from
   `siphasher` (reviewed, maintained, no dependencies); the four-lane
   composition is in-house. **Stated per OMNI-CORE 11f**: this is an
   in-house composition of a reviewed primitive — suitable for tamper
   detection between cooperating components, not a substitute for a
   reviewed AEAD where an adversary controls the channel. Anything that
   crosses a hostile wire gets AEAD in a later ADR.
2. **Generic ring**: closed by decision, not code. In a workspace that
   denies `unsafe`, a safe owned-slot SPSC is not hand-rollable without
   duplicating crossbeam's internals — which the estate forbids
   (never duplicate a reviewed primitive). Generic-payload transport is
   `omni-ring::mpmc::MpmcRing` used under single-producer/single-consumer
   discipline; the hand-rolled `u64` ring remains the Lean-provable
   specialization for descriptor transport (§161–§163, §181).
3. **Epoch is part of the sealed content**: `bump_epoch` after `seal`
   invalidates the authenticator (capability re-grants must re-seal).

## Consequences

- **Good**: `auth_mac` is a first-class seal/verify contract with tamper
  tests; REQ-OS-002 has no deferred remainder.
- **Good**: the generic-ring question is answered by a documented
  boundary instead of an unsafe experiment or a duplicate primitive.
- **Cost**: the MAC covers exactly the five content fields; extra fields
  (owner tags, memory-class bits) require an ADR to join the canonical
  bytes — silently extending the MAC input would break stored tokens.
- **Risk**: the four-lane construction is unreviewed; if this token ever
  authenticates across a trust boundary, the first change is swapping in
  a reviewed AEAD, not patching lanes.
