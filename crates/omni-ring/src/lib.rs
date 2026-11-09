//! Lock-free rings for descriptor transport (REQ-OS-001).
//!
//! Two primitives, with the formal-vs-empirical distinction stated per the
//! estate formal-verification invariant (OMNI-CORE 11f):
//!
//! - [`spsc::SpscRing`] — hand-rolled, **wait-free** bounded SPSC ring over
//!   `u64` descriptors. Cache-line-padded cursors, pure atomics, no
//!   `unsafe`. Index masking and bounded-step behavior are machine-checked
//!   in `proofs/` (`ring_index_bounded`, goals.md §181).
//! - [`mpmc::MpmcRing`] — **lock-free** bounded MPMC ring delegated to
//!   `crossbeam_queue::ArrayQueue` (estate precedent: actor-kit, ferro).
//!   Lock-free, not wait-free; that distinction is deliberate and honest.
//!
//! Descriptor-first design note: the goals corpus (§161–§163) carries DMA
//! descriptors as 64-bit tokens; the `u64` SPSC ring is the Lean-provable
//! specialization of that contract. Generic-payload transport is satisfied
//! by [`mpmc::MpmcRing`] under single-producer/single-consumer discipline
//! (ADR-0012): in a `no-unsafe` workspace a safe owned-slot SPSC is not
//! hand-rollable, and duplicating crossbeam's internals is not on the
//! table (estate rule: never duplicate a reviewed primitive).

// Box/Vec for the slot storage come from `alloc` explicitly so the SPSC
// module stays portable to `no_std` + alloc targets if the MPMC leg is
// feature-gated out later.
extern crate alloc;

pub mod mpmc;
pub mod spsc;
