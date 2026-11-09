//! omniosk — sovereign OS seed (K4 in `docs/goals/index.md`).
//!
//! Phase 2 begins library-first: everything that can be written, tested,
//! and gated on the host exists as a `no_std` module; the boot assembly and
//! kernel binary land next per ADR-0013's decided path (multiboot → long
//! mode → `kernel_main`, QEMU `-kernel` harness with isa-debug-exit).
//!
//! Hardware register access is abstracted behind [`serial::PortIo`] so the
//! byte-exact init/write sequences are host-tested against a mock, while
//! the real port I/O stays `x86_64`-gated inline assembly.

#![cfg_attr(not(test), no_std)]

pub mod qemu_exit;
pub mod serial;
