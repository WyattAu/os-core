//! Lock-free rings for descriptor transport (REQ-OS-001).
//!
//! Two primitives, with the formal-vs-empirical distinction stated per the
//! estate formal-verification invariant (OMNI-CORE 11f):
//!
//! - [`spsc::SpscRing`] — hand-rolled, **wait-free** bounded SPSC ring over
//!   `u64` descriptors. Cache-line-padded cursors, pure atomics, no
//!   `unsafe`. Index masking and bounded-step behavior are machine-checked
//!   in `proofs/` (`ring_index_bounded`, goals.md §181).
//! - [`slot::SlotTable`] — the generic-payload follow-up (REQ-OS-001),
//!   resolved at the architecture layer: rings carry `u64` descriptors,
//!   the table stores typed payloads. `io_uring`'s model; keeps the
//!   `unsafe_code = deny` contract intact.
//! - [`mpmc::MpmcRing`] — **lock-free** bounded MPMC ring delegated to
//!   `crossbeam_queue::ArrayQueue` (estate precedent: actor-kit, ferro).
//!   Lock-free, not wait-free; that distinction is deliberate and honest.
//!
//! Descriptor-first design note: the goals corpus (§161–§163) carries DMA
//! descriptors as 64-bit tokens; the `u64` SPSC ring is the Lean-provable
//! specialization of that contract — and it is allocation-free (inline
//! slot array), so it runs in `no_std`-no-alloc kernels (omniosk, ADR-0013).
//! Generic-payload transport is satisfied by [`mpmc::MpmcRing`] under
//! single-producer/single-consumer discipline (ADR-0012): in a `no-unsafe`
//! workspace a safe owned-slot SPSC is not hand-rollable, and duplicating
//! crossbeam's internals is not on the table (estate rule: never duplicate
//! a reviewed primitive).
//!
//! `no_std` note: the crate links `std` only through the `mpmc` feature
//! (crossbeam). With `default-features = false` it is a pure-`core` crate
//! suitable for `x86_64-unknown-none` kernels (ADR-0013).

#![cfg_attr(not(feature = "mpmc"), no_std)]

#[cfg(feature = "mpmc")]
pub mod mpmc;
pub mod slot;
pub mod spsc;
