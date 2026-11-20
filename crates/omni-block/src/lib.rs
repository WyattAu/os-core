//! # omni-block — block-device seam (REQ layer over REQ-OS-006)
//!
//! The `NVMe` path in `kernels/omniosk-kernel/src/nvme.rs` proved one thing
//! end-to-end: WRITE a sector, read it back, byte-compare in-kernel
//! (`NVME WRITE PASS`, the QEMU smoke gate). This crate lifts that path
//! into a contract any block device can satisfy, so the property tests
//! and the smoke semantics are defined once, against a device model,
//! not re-derived per driver.
//!
//! **The seam generalized from two implementations.** [`BlockRam`] is
//! the in-memory reference device; the `NVMe` queue pair in
//! `omniosk-kernel` implements the same [`BlockDevice`] contract over
//! its I/O queues (its `read_lba`/`write_lba` are the exact calls the
//! smoke exercised before the trait existed). The trait is the
//! contract, not a vtable mandate: consumers can stay generic over
//! `B: BlockDevice` (static dispatch, no `alloc`) — `&mut dyn` remains
//! available where DMA constraints permit. [`BlockRam`] never
//! allocates and never panics.
//!
//! Invariants (asserted by `tests/model.rs`):
//!
//! 1. `read_lba`/`write_lba` are bounds-checked — an out-of-range LBA
//!    is [`BlockError::OutOfRange`], never a panic (workspace denies
//!    the panic family).
//! 2. A write is durable within the device: write then read back
//!    yields the same 512 bytes ([`BlockRam::verify_roundtrip`] is the
//!    kernel smoke as a library call).
//! 3. Writes to one LBA never disturb another (sector independence).
//! 4. A fresh device reads as all zeros.

#![no_std]
#![deny(unsafe_code)]

/// Sector size of the seam, in bytes: the ATA/ATAPI LBA the `NVMe`
/// namespace presents and the kernel smoke writes.
pub const LBA_SIZE: usize = 512;

/// Every fallible block operation's failure mode. No panics: the estate
/// posture denies the panic family, and a microcore cannot unwind on
/// the storage path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    /// The LBA is beyond the device's capacity (or not representable
    /// as an offset on this platform).
    OutOfRange,
    /// A required alignment does not hold. Reserved for the `NVMe`
    /// implementation (DMA buffer alignment); `BlockRam` operations
    /// are sector-aligned by construction and never produce this.
    Misaligned,
    /// The device failed to retain a verified write: the roundtrip
    /// byte-compare did not match.
    Io,
    /// The device cannot perform the requested operation (e.g. a
    /// read-only medium exposed to a write).
    NotSupported,
}

/// An in-memory block device: `SECTORS` LBAs of [`LBA_SIZE`] bytes,
/// zero-initialized, owned inline (a `static` in the kernel, a stack
/// value in tests). No methods allocate; no methods panic.
///
/// The backing store is an array of sectors, not a flat byte array:
/// the const parameter stays a standalone array length (stable Rust
/// rejects `SECTORS * LBA_SIZE` as a length without
/// `generic_const_exprs`), and per-sector bounds checks need no
/// arithmetic.
#[derive(Debug, Clone, Copy)]
pub struct BlockRam<const SECTORS: usize> {
    sectors: [[u8; LBA_SIZE]; SECTORS],
}

impl<const SECTORS: usize> BlockRam<SECTORS> {
    /// Creates an all-zero device of `SECTORS` sectors.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sectors: [[0; LBA_SIZE]; SECTORS],
        }
    }

    /// The device capacity in logical blocks (`SECTORS`).
    #[must_use]
    pub const fn capacity_lbas(&self) -> usize {
        SECTORS
    }

    /// Reads sector `lba` into `out`, overwriting all [`LBA_SIZE`]
    /// bytes.
    ///
    /// # Errors
    ///
    /// [`BlockError::OutOfRange`] if `lba >= SECTORS` (or unhelpfully
    /// large for the platform's `usize`).
    pub fn read_lba(&self, lba: u64, out: &mut [u8; LBA_SIZE]) -> Result<(), BlockError> {
        let i = Self::sector_index(lba)?;
        let sector = self.sectors.get(i).ok_or(BlockError::OutOfRange)?;
        out.copy_from_slice(sector);
        Ok(())
    }

    /// Writes the [`LBA_SIZE`] bytes of `data` to sector `lba`.
    ///
    /// # Errors
    ///
    /// [`BlockError::OutOfRange`] if `lba >= SECTORS` (or unhelpfully
    /// large for the platform's `usize`).
    pub fn write_lba(&mut self, lba: u64, data: &[u8; LBA_SIZE]) -> Result<(), BlockError> {
        let i = Self::sector_index(lba)?;
        let sector = self.sectors.get_mut(i).ok_or(BlockError::OutOfRange)?;
        *sector = *data;
        Ok(())
    }

    /// Sector index for `lba`, bounds-checked without indexing.
    fn sector_index(lba: u64) -> Result<usize, BlockError> {
        let i = usize::try_from(lba).map_err(|_| BlockError::OutOfRange)?;
        if i >= SECTORS {
            return Err(BlockError::OutOfRange);
        }
        Ok(i)
    }
}

/// The block-device contract (REQ layer): LBA-addressed sector
/// transport of exactly [`LBA_SIZE`] bytes per operation.
///
/// Implementations: [`BlockRam`] (the in-memory reference) and the
/// `NVMe` session in `omniosk-kernel` (the bare-metal path the QEMU
/// smoke gates). Consumers stay generic over `B: BlockDevice`.
pub trait BlockDevice {
    /// The device capacity in logical blocks.
    fn capacity_lbas(&self) -> u64;

    /// Reads sector `lba` into `out`.
    ///
    /// # Errors
    ///
    /// Implementation-defined mapping of [`BlockError`]; `OutOfRange`
    /// for an LBA beyond the capacity.
    fn read_lba(&mut self, lba: u64, out: &mut [u8; LBA_SIZE]) -> Result<(), BlockError>;

    /// Writes `data` to sector `lba`.
    ///
    /// # Errors
    ///
    /// Implementation-defined mapping of [`BlockError`]; `OutOfRange`
    /// for an LBA beyond the capacity.
    fn write_lba(&mut self, lba: u64, data: &[u8; LBA_SIZE]) -> Result<(), BlockError>;

    /// The kernel's WRITE/read-back smoke (REQ-OS-006, the
    /// `NVME WRITE PASS` semantics) as a library call: write `data` to
    /// `lba`, read the sector back, byte-compare.
    ///
    /// This is the smoke contract every block implementation must
    /// satisfy before the QEMU gate asserts it against real hardware.
    ///
    /// # Errors
    ///
    /// [`BlockError::OutOfRange`] from the write or read if `lba` is
    /// out of bounds; [`BlockError::Io`] if the read-back does not
    /// match what was written.
    fn verify_roundtrip(&mut self, lba: u64, data: &[u8; LBA_SIZE]) -> Result<(), BlockError> {
        self.write_lba(lba, data)?;
        let mut readback = [0; LBA_SIZE];
        self.read_lba(lba, &mut readback)?;
        if readback == *data {
            Ok(())
        } else {
            Err(BlockError::Io)
        }
    }
}

impl<const SECTORS: usize> BlockDevice for BlockRam<SECTORS> {
    fn capacity_lbas(&self) -> u64 {
        SECTORS as u64
    }

    fn read_lba(&mut self, lba: u64, out: &mut [u8; LBA_SIZE]) -> Result<(), BlockError> {
        BlockRam::read_lba(self, lba, out)
    }

    fn write_lba(&mut self, lba: u64, data: &[u8; LBA_SIZE]) -> Result<(), BlockError> {
        BlockRam::write_lba(self, lba, data)
    }
}

impl<const SECTORS: usize> Default for BlockRam<SECTORS> {
    fn default() -> Self {
        Self::new()
    }
}

/// Pure `NVMe` wire-layout helpers (REQ-OS-006): the byte-order
/// contracts machine-checked in `proofs/Proofs.lean` live here as
/// functions the kernel consumes, so the wire arithmetic is exercised
/// by the refinement oracle (`tests/refinement.rs`, ADR-0014 pattern)
/// instead of being inlined untested in ISR code.
pub mod wire {
    /// CQE DW3 phase bit. DW3[31:16] is the status half
    /// (`SF << 1 | P`), so the phase is DW3 bit 16 — NOT bit 0, which
    /// is the CID's LSB. Reading bit 0 is the exact bug the theorem
    /// `nvme_cqe_phase_is_high_half_lsb` documents (commit 66e7d4b:
    /// odd-CID commands appeared to complete, even-CID never did).
    #[must_use]
    pub const fn cqe_phase_of(dw3: u32) -> u32 {
        (dw3 >> 16) & 1
    }

    /// CQE DW3 status half (DW3[31:16], bytes 14-15 LE).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // exact: the masked top half IS a u16
    pub const fn cqe_status(dw3: u32) -> u16 {
        (dw3 >> 16) as u16
    }

    /// CQE DW3 command id half (DW3[15:0], bytes 12-13 LE).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // exact: the masked low half IS a u16
    pub const fn cqe_cid(dw3: u32) -> u16 {
        dw3 as u16
    }

    /// `CreateCq` / `CreateSq` CDW10 overlay: `ID[15:0] | QSIZE[31:16]`,
    /// where QSIZE is the *0-based* entry count (quantity - 1) per the
    /// QEMU `NvmeCreateCq` overlay — theorems
    /// `nvme_create_cq_cqid_extraction` and
    /// `nvme_create_cq_qsize_extraction`. Writing the 1-based count
    /// across the whole word was the stale `cqid=3, qsize=0` trace.
    #[must_use]
    #[allow(clippy::cast_lossless)] // From<u16> is not const-callable on stable
    pub const fn create_q_cdw10(cqid: u16, qsize0: u16) -> u32 {
        (cqid as u32) | ((qsize0 as u32) << 16)
    }
}

#[cfg(test)]
mod tests {
    // Tests link the harness's std; the lib itself stays no_std.
    extern crate std;

    use super::*;

    #[test]
    fn fresh_device_is_zeroed_and_bounds_are_checked() {
        let disk = BlockRam::<4>::new();
        assert_eq!(disk.capacity_lbas(), 4);
        let mut buf = [0xAA; LBA_SIZE];
        assert!(disk.read_lba(3, &mut buf).is_ok());
        assert!(buf.iter().all(|&b| b == 0), "a fresh device reads as zeros");
        assert_eq!(disk.read_lba(4, &mut buf), Err(BlockError::OutOfRange));
        assert_eq!(
            disk.read_lba(u64::MAX, &mut buf),
            Err(BlockError::OutOfRange)
        );
    }

    #[test]
    fn roundtrip_smoke_passes_on_the_model_device() {
        let mut disk = BlockRam::<8>::new();
        let mut payload = [0u8; LBA_SIZE];
        let mut i = 0u8;
        for b in &mut payload {
            *b = i;
            i = i.wrapping_add(1);
        }
        assert_eq!(disk.verify_roundtrip(1, &payload), Ok(()));
    }
}
