# ADR-17: HFT data plane first design — server + tick store

- **Status**: Accepted
- **Date**: 2026-11-20

## Context

The HFT pillars (XXII–XXVI) fund the program, and their building
blocks now exist: omni-io (doorbell ingestion), omni-sbe (the CME
MDP 3.0 codec), omni-ring (the transport), omni-task (the job model),
and omni-service (the HTTP posture template). What does not exist is
the design layer between them — the server that hosts the ingestion
and query surface, and the database that persists the ticks. "First
design" means: decisions recorded, crates sketched, formats fixed —
not production hardening.

## Decision

1. **Two crates, one data plane.**
   - `omni-tick` — the database. A fixed-width 32-byte tick record
     (`ts_ns` u64, `instrument` u32, `flags` u32, `bid_px` u64,
     `ask_px` u64, LE wire format) inside a segment container: 16-byte
     header (magic, version, record size, count) followed by records,
     append-only, replay = sequential scan. No WAL, no LSM, no B-trees
     — the HFT discipline is append + mmap + scan, and the store is
     the durability contract for replay, not an OLTP engine.
   - `omni-md` — the server. Ingestion (omni-io socket reads) →
     decode (omni-sbe) → fan-out (omni-ring) → persist (omni-tick),
     with the query surface on omni-service's axum posture
     (`/healthz`, `/readyz`, then range/latest endpoints). Lands as
     the next tranche; this ADR fixes the seams.
2. **The record format is the database's first contract.** Fixed
   width, little-endian, no in-place mutation: a tick is written once
   and replayed in order. Segment integrity in v0.1 is header magic +
   record count; CRC/trailer is a tracked follow-up.
3. **Ingestion is the omni-io hot path.** SQPOLL + registered buffers
   (ADR-0016's tranche) are the default configuration for the feed
   socket; the syscall-free submit loop is the latency budget's core.
4. **Sequencing.** omni-tick skeleton lands with this ADR (the format
   is testable without a feed); omni-md lands next; UDP ingestion on
   the A53 is the hardware-session item; the FPGA front-end (K5)
   queues behind the board decision as always.

## Consequences

- **Good**: the HFT pillars have a concrete data plane — every piece
  of it (ingest, decode, transport, store, serve) is an estate crate
  with a REQ row and a gate.
- **Good**: the tick format is fixed early, so omni-md, the store, and
  any FPGA front-end all compile against the same 32-byte contract.
- **Cost**: fixed-width ticks exclude variable-length payloads (SBE
  varData fields) — those route through a separate sidecar path in
  omni-md, not through omni-tick.
- **Risk**: "append + scan" replay cost grows linearly with segment
  history; the mitigation (segment index/range trees) is a Phase-3
  tranche, deferred until the feed volume demands it.

**Update 2026-11-21 (CRC trailer shipped):** the segment format gains
the tracked integrity feature — a CRC32 (IEEE, `crc32fast`) trailer in
the buffer's last 4 bytes, streamed per push by `crc32fast::Hasher`
and verified on `open`. Bit rot and truncation are rejected, never
replayed. Format version 2; omni-tick 0.2.0 (breaking, honest semver —
the Tick type crosses omni-md/omni-md-server, both bumped to 0.2.0).
Boundary note unchanged: CRC detects rot, it is not an adversarial MAC.
