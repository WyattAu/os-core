//! Emulated NVMe access (ADR-0011 first device step).
//!
//! QEMU's `-device nvme` (Red Hat 1b36:0010) sits on PCI bus 0; SeaBIOS
//! assigned its BAR0 before the handoff, covered by the boot identity map
//! (512 GiB of 1 GiB pages). The driver speaks raw NVMe over MMIO:
//! disable controller, program the admin queues, Identify (model printed
//! over COM1), create I/O queues, read LBA 0, write a pattern to LBA 1,
//! read it back, and verify the drive's data actually round-tripped.
//!
//! Everything here is `unsafe` by package policy (ADR-0013 boundary).

use omniosk::serial::{PortIo, Serial, COM1};

const PCI_NVME_VENDOR: u16 = 0x1B36;
const PCI_NVME_DEVICE: u16 = 0x0010;

/// Page-aligned scratch: admin queues, identify data, I/O queues, read data.
#[repr(C, align(4096))]
/// The six DMA pages the smoke needs, supplied by the caller (the
/// microcore heap — REQ-OS-007). All addresses must be 4 KiB-aligned
/// and the pages zeroed before `smoke` runs.
pub struct NvmePages {
    pub asq: usize,
    pub acq: usize,
    pub ident: usize,
    pub readb: usize,
    pub iosq: usize,
    pub iocq: usize,
}

const QSIZE: usize = 4;
/// I/O queue capacity. DISCOVERY (2026-11-14): QEMU rejects an SQ/CQ
/// doorbell value >= queue size (`new_tail >= sq->size` in
/// nvme_process_db) — with 4-entry I/O queues the 4th submission's
/// doorbell (4) is rejected and the queue never completes again.
/// 64 entries give a session ample submission headroom (MQES is 255+).
const IO_QSIZE: usize = 64;
pub const MAGIC: &[u8] = b"OMNIOSK-NVME-OK!";

/// The NSZE parsed from Identify-Namespace (diagnostics surface).
pub static LAST_NSZE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

#[inline]
pub(crate) fn mmio32(base: usize, off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}

#[inline]
fn mmio32w(base: usize, off: usize, v: u32) {
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, v) }
}

#[inline]
fn mmio64w(base: usize, off: usize, v: u64) {
    mmio32w(base, off, v as u32);
    mmio32w(base, off + 4, (v >> 32) as u32);
}

#[inline]
fn vbyte(base: usize, off: usize) -> u8 {
    unsafe { core::ptr::read_volatile((base + off) as *const u8) }
}

#[inline]
fn wbyte(base: usize, off: usize, v: u8) {
    unsafe { core::ptr::write_volatile((base + off) as *mut u8, v) }
}

/// 32-bit PCI configuration read through 0xCF8/0xCFC (bus 0).
fn cfg32(bus: u32, dev: u32, off: u32) -> u32 {
    let addr = 0x8000_0000 | (bus << 16) | (dev << 11) | (off & 0xFC);
    let mut ports = RawPorts;
    ports.out32(0xCF8, addr);
    ports.in32(0xCFC)
}

/// Writes a 64-byte NVMe command entry at SQ `tail`, then rings the SQ
/// tail doorbell with the NEW tail pointer (entry count after submission).
fn submit(bar: usize, sq: usize, tail: usize, sq_db_off: usize, entry: &[u8; 64]) {
    let slot = sq + tail * 64;
    for (i, chunk) in entry.chunks_exact(4).enumerate() {
        let v = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        mmio32w(slot, i * 4, v);
    }
    mmio32w(bar, sq_db_off, (tail as u32) + 1);
}

/// Polls the CQE at `idx` until its phase bit matches `phase` (bounded);
/// returns Some(dw3) on completion, None on timeout. Rings the CQ head
/// doorbell (`cq_hd_off`) for the consumed entry — wrapped by `qsize`,
/// the RING'S OWN size (admin 32, I/O 64): wrapping against the wrong
/// queue's size rewinds the head and corrupts the device's accounting.
///
/// CQE DW3 layout (per Linux nvme_completion / QEMU NvmeCqe — verified
/// against QEMU 11.1.3 hw/nvme): DW3[15:0] = CID, DW3[31:16] = status
/// u16 with SF<<1 | P. The phase bit is therefore bit 16 of DW3, NOT
/// bit 0. Reading bit 0 reads the CID's LSB: odd CIDs (identify, CID=1)
/// appear to complete, even CIDs (create-cq, CID=2) never do.
fn wait_cqe(
    bar: usize,
    cq: usize,
    idx: usize,
    phase: u32,
    cq_hd_off: usize,
    qsize: usize,
) -> Option<u32> {
    let mut spins = 0usize;
    let dw3 = loop {
        let v = mmio32(cq, idx * 16 + 12);
        if wire::cqe_phase_of(v) == phase {
            break v;
        }
        spins += 1;
        if spins > 50_000_000 {
            return None;
        }
    };
    mmio32w(bar, cq_hd_off, ((idx + 1) % qsize) as u32);
    Some(dw3)
}

fn cq_hd_off(qid: u32, stride: u32) -> usize {
    0x1000 + ((2 * qid + 1) as usize) * (stride as usize)
}

fn sq_db_off(qid: u32, stride: u32) -> usize {
    0x1000 + ((2 * qid) as usize) * (stride as usize)
}

fn write_hex(uart: &mut Serial<RawPorts>, v: u32) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    for shift in [28, 24, 20, 16, 12, 8, 4, 0] {
        uart.write_byte(HEX[((v >> shift) & 0xF) as usize]);
    }
}

/// Live device state after `init`: the BAR, the DMA pages, and the
/// queue cursors the I/O operations advance. Built by `smoke` once the
/// I/O queues exist; consumed by `read_lba`/`write_lba`.
pub struct NvmeSession {
    pub bar: usize,
    /// Device-reported LBA count (NSZE from Identify-Namespace).
    pub lbas: u64,
    pub asq: usize,
    pub acq: usize,
    pub iosq: usize,
    pub iocq: usize,
    pub readb: usize,
    pub(crate) stride: u32,
    /// Next I/O submission-queue slot.
    pub(crate) iosq_tail: usize,
    /// Next I/O completion-queue entry to poll.
    pub(crate) cqe_idx: usize,
    /// Completion phase bit for `cqe_idx`.
    pub(crate) cqe_phase: u32,
}

/// Submits an I/O read of `lba` into the session's read page and waits
/// for the completion. False on timeout or status error.
fn io_read_lba(s: &mut NvmeSession, lba: u64) -> bool {
    let mut sqe = [0u8; 64];
    sqe[0] = 0x02; // Read
    sqe[4..8].copy_from_slice(&1u32.to_le_bytes()); // NSID 1
    sqe[24..32].copy_from_slice(&(s.readb as u64).to_le_bytes()); // PRP1
    sqe[40..44].copy_from_slice(&(lba as u32).to_le_bytes()); // CDW10: SLBA lo
    sqe[44..48].copy_from_slice(&((lba >> 32) as u32).to_le_bytes()); // CDW11: SLBA hi
    sqe[48..52].copy_from_slice(&0u32.to_le_bytes()); // CDW12: NLB-1 = 0
    let tail = s.iosq_tail;
    submit(s.bar, s.iosq, tail, sq_db_off(1, s.stride), &sqe);
    s.iosq_tail = (tail + 1) % IO_QSIZE;
    let ok = wait_cqe(s.bar, s.iocq, s.cqe_idx, s.cqe_phase, cq_hd_off(1, s.stride), IO_QSIZE)
        .is_some();
    s.cqe_idx += 1;
    if s.cqe_idx >= IO_QSIZE {
        s.cqe_idx = 0;
        s.cqe_phase = s.cqe_phase.wrapping_add(1) & 1;
    }
    ok
}

/// Submits an I/O write of the session's read page to `lba` and waits
/// for the completion. False on timeout or status error.
fn io_write_lba(s: &mut NvmeSession, lba: u64) -> bool {
    let mut sqe = [0u8; 64];
    sqe[0] = 0x01; // Write
    sqe[4..8].copy_from_slice(&1u32.to_le_bytes()); // NSID 1
    sqe[24..32].copy_from_slice(&(s.readb as u64).to_le_bytes()); // PRP1
    sqe[40..44].copy_from_slice(&(lba as u32).to_le_bytes()); // CDW10: SLBA lo
    sqe[44..48].copy_from_slice(&((lba >> 32) as u32).to_le_bytes()); // CDW11: SLBA hi
    sqe[48..52].copy_from_slice(&0u32.to_le_bytes()); // CDW12: NLB-1 = 0
    let tail = s.iosq_tail;
    submit(s.bar, s.iosq, tail, sq_db_off(1, s.stride), &sqe);
    s.iosq_tail = (tail + 1) % IO_QSIZE;
    let ok = wait_cqe(s.bar, s.iocq, s.cqe_idx, s.cqe_phase, cq_hd_off(1, s.stride), IO_QSIZE)
        .is_some();
    s.cqe_idx += 1;
    if s.cqe_idx >= IO_QSIZE {
        s.cqe_idx = 0;
        s.cqe_phase = s.cqe_phase.wrapping_add(1) & 1;
    }
    ok
}

use omni_block::{wire, BlockDevice, BlockError};

/// The `omni-block` contract over the NVMe I/O queues (REQ layer).
impl BlockDevice for NvmeSession {
    fn capacity_lbas(&self) -> u64 {
        self.lbas
    }

    fn read_lba(&mut self, lba: u64, out: &mut [u8; 512]) -> Result<(), BlockError> {
        if lba >= self.capacity_lbas() {
            return Err(BlockError::OutOfRange);
        }
        if !io_read_lba(self, lba) {
            return Err(BlockError::Io);
        }
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = vbyte(self.readb, i);
        }
        Ok(())
    }

    fn write_lba(&mut self, lba: u64, data: &[u8; 512]) -> Result<(), BlockError> {
        if lba >= self.capacity_lbas() {
            return Err(BlockError::OutOfRange);
        }
        for (i, b) in data.iter().enumerate() {
            wbyte(self.readb, i, *b);
        }
        if !io_write_lba(self, lba) {
            return Err(BlockError::Io);
        }
        Ok(())
    }
}

/// Full device bring-up + smoke. Serial carries per-stage progress and the
/// controller's model number. Returns the live session on success (the
/// caller may then use `read_lba`/`write_lba`); None on any failure.
pub fn smoke(p: &NvmePages) -> Option<NvmeSession> {
    let mut uart = Serial::new(RawPorts, COM1);

    // ---- PCI: find the QEMU NVMe on bus 0 ----
    let mut bar = 0usize;
    for dev in 0u32..32 {
        let id = cfg32(0, dev, 0x00);
        if (id & 0xFFFF) == u32::from(PCI_NVME_VENDOR)
            && ((id >> 16) & 0xFFFF) == u32::from(PCI_NVME_DEVICE)
        {
            bar = (cfg32(0, dev, 0x10) & 0xFFFF_FFF0) as usize;
            break;
        }
    }
    if bar == 0 {
        uart.write_str("NVME: device not found\n");
        return None;
    }

    // Doorbell stride = 4 bytes (SeaBIOS-verified: CQ0HDBL @ 0x1004).
    // The CAP[2:0] read of 0x7 does not reflect the effective stride
    // for this QEMU/SeaBIOS pair. The 512B stride broke the CQ head
    // doorbell, preventing the controller from advancing past the
    // identify completion (the admin CQ appeared full).
    let stride = 4u32;
    let cq0_hd = cq_hd_off(0, stride); // = 0x1004

    // ---- controller disable + queue programming ----
    mmio32w(bar, 0x14, 0); // CC.EN = 0
    let mut spins = 0usize;
    while mmio32(bar, 0x1C) & 1 != 0 {
        spins += 1;
        if spins > 100_000_000 {
            uart.write_str("NVME: disable timeout\n");
            return None;
        }
    }

    // SAFETY (package boundary, ADR-0013): DMA target buffers, statics.
    let (asq, acq) = (p.asq, p.acq);
    let (ident, readb) = (p.ident, p.readb);
    let (iosq, iocq) = (p.iosq, p.iocq);

    // AQA: 32-entry admin queues (DISCOVERY 2026-11-14: QEMU rejects
    // doorbell values >= queue size — 4-entry admin queues would die on
    // the 4th admin submission, the Identify-Namespace below).
    const ADMIN_QSIZE: u32 = 32;
    mmio32w(bar, 0x24, ((ADMIN_QSIZE - 1) << 16) | (ADMIN_QSIZE - 1)); // AQA
    mmio64w(bar, 0x28, asq as u64); // ASQ
    mmio64w(bar, 0x30, acq as u64); // ACQ
    // CC: IOSQES = 6 (64B SQEs), IOCQES = 4 (16B CQEs), EN = 1.
    mmio32w(bar, 0x14, (6 << 16) | (4 << 20) | 1);
    spins = 0;
    while mmio32(bar, 0x1C) & 1 == 0 {
        spins += 1;
        if spins > 100_000_000 {
            uart.write_str("NVME: enable timeout\n");
            return None;
        }
    }

    // ---- admin: Identify Controller (op 06, CNS 1) ----
    // SQE layout (QEMU NvmeCmd): opcode 0, flags 1, cid 2, nsid 4,
    // mptr 16, prp1 24, prp2 32, cdw10 40, cdw11 44, cdw12 48.
    let mut sqe = [0u8; 64];
    sqe[0] = 0x06; // opcode
    sqe[2..4].copy_from_slice(&1u16.to_le_bytes()); // CID 1
    sqe[24..32].copy_from_slice(&(ident as u64).to_le_bytes()); // PRP1
    sqe[40..44].copy_from_slice(&1u32.to_le_bytes()); // CDW10: CNS = 1
    submit(bar, asq, 0, sq_db_off(0, stride), &sqe);
    let dw3 = match wait_cqe(bar, acq, 0, 1, cq0_hd, ADMIN_QSIZE as usize) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: identify CQE timeout\n");
            return None;
        }
    };
    if (dw3 >> 17 & 0x7FFF) != 0 {
        uart.write_str("NVME: identify status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return None;
    }

    // Model number: bytes 24..88, printed trimmed.
    uart.write_str("NVME model: ");
    let mut started = false;
    for i in 24..88 {
        let c = vbyte(ident, i);
        if c == 0 {
            break;
        }
        if c != b' ' || started {
            started = true;
            uart.write_byte(c);
        }
    }
    uart.write_str("\nNVME IDENT PASS\n");

    // ---- admin: Identify Namespace (op 06, CNS 0, NSID 1) ----
    // NSZE (bytes 0-7 of the namespace data) is the device-reported
    // LBA count: the honest capacity for the omni-block trait.
    let mut sqe = [0u8; 64];
    sqe[0] = 0x06; // opcode
    sqe[2..4].copy_from_slice(&4u16.to_le_bytes()); // CID 4
    sqe[4..8].copy_from_slice(&1u32.to_le_bytes()); // NSID 1
    sqe[24..32].copy_from_slice(&(ident as u64).to_le_bytes()); // PRP1
    sqe[40..44].copy_from_slice(&0u32.to_le_bytes()); // CDW10: CNS = 0
    submit(bar, asq, 1, sq_db_off(0, stride), &sqe);
    let dw3 = match wait_cqe(bar, acq, 1, 1, cq0_hd, ADMIN_QSIZE as usize) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: identify-ns CQE timeout\n");
            return None;
        }
    };
    if (dw3 >> 17 & 0x7FFF) != 0 {
        uart.write_str("NVME: identify-ns status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return None;
    }
    let nsze = u64::from_le_bytes([
        vbyte(ident, 0),
        vbyte(ident, 1),
        vbyte(ident, 2),
        vbyte(ident, 3),
        vbyte(ident, 4),
        vbyte(ident, 5),
        vbyte(ident, 6),
        vbyte(ident, 7),
    ]);
    LAST_NSZE.store(nsze, core::sync::atomic::Ordering::Relaxed);

    // ---- I/O queues: CQ1 then SQ1 ----
    let mut sqe = [0u8; 64];
    sqe[0] = 0x05; // Create IOCQ
    sqe[2..4].copy_from_slice(&2u16.to_le_bytes()); // CID 2
    sqe[24..32].copy_from_slice(&(iocq as u64).to_le_bytes()); // PRP1
    let cdw10 = wire::create_q_cdw10(1, (IO_QSIZE as u16) - 1);
    sqe[40..44].copy_from_slice(&cdw10.to_le_bytes()); // CQID[15:0] | QSIZE0[31:16]
    sqe[44..46].copy_from_slice(&3u16.to_le_bytes()); // flags: PC + IEN
    sqe[46..48].copy_from_slice(&0u16.to_le_bytes()); // IRQ vector = 0
    submit(bar, asq, 2, sq_db_off(0, stride), &sqe);
    let dw3 = match wait_cqe(bar, acq, 2, 1, cq0_hd, ADMIN_QSIZE as usize) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: IOCQ create timeout\n");
            uart.write_str("CC=");
            write_hex(&mut uart, mmio32(bar, 0x14));
            uart.write_str(" CSTS=");
            write_hex(&mut uart, mmio32(bar, 0x1C));
            uart.write_str(" acq[16..32]: ");
            for i in 16..32 {
                write_hex(&mut uart, vbyte(acq, i) as u32);
            }
            uart.write_str(" sq[64..96]: ");
            for i in 64..96 {
                write_hex(&mut uart, vbyte(asq, i) as u32);
            }
            uart.write_str("\n");
            return None;
        }
    };
    if (dw3 >> 17 & 0x7FFF) != 0 {
        uart.write_str("NVME: IOCQ status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return None;
    }

    let mut sqe = [0u8; 64];
    sqe[0] = 0x01; // Create IOSQ
    sqe[2..4].copy_from_slice(&3u16.to_le_bytes()); // CID 3
    sqe[24..32].copy_from_slice(&(iosq as u64).to_le_bytes()); // PRP1
    let cdw10 = wire::create_q_cdw10(1, (IO_QSIZE as u16) - 1);
    sqe[40..44].copy_from_slice(&cdw10.to_le_bytes()); // SQID[15:0] | QSIZE0[31:16]
    sqe[44..46].copy_from_slice(&1u16.to_le_bytes()); // SQ_FLAGS: PC
    sqe[46..48].copy_from_slice(&1u16.to_le_bytes()); // CQID = 1
    submit(bar, asq, 3, sq_db_off(0, stride), &sqe);
    let dw3 = match wait_cqe(bar, acq, 3, 1, cq0_hd, ADMIN_QSIZE as usize) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: IOSQ create timeout\n");
            return None;
        }
    };
    if (dw3 >> 17 & 0x7FFF) != 0 {
        uart.write_str("NVME: IOSQ status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return None;
    }

    // ---- I/O: read LBA 0 into the read buffer ----
    let mut sqe = [0u8; 64];
    sqe[0] = 0x02; // Read
    sqe[2..4].copy_from_slice(&4u16.to_le_bytes()); // CID 4
    sqe[4..8].copy_from_slice(&1u32.to_le_bytes()); // NSID 1
    sqe[24..32].copy_from_slice(&(readb as u64).to_le_bytes()); // PRP1
    sqe[40..44].copy_from_slice(&0u32.to_le_bytes()); // CDW10: SLBA = 0
    sqe[48..52].copy_from_slice(&0u32.to_le_bytes()); // CDW12: NLB-1 = 0
    submit(bar, iosq, 0, sq_db_off(1, stride), &sqe);
    let dw3 = match wait_cqe(bar, iocq, 0, 1, cq_hd_off(1, stride), IO_QSIZE) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: read timeout\n");
            return None;
        }
    };
    if (dw3 >> 17 & 0x7FFF) != 0 {
        uart.write_str("NVME: read status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return None;
    }

    for (i, expected) in MAGIC.iter().enumerate() {
        if vbyte(readb, i) != *expected {
            uart.write_str("NVME: data mismatch\n");
            return None;
        }
    }

    // ---- I/O: WRITE a pattern to LBA 1, read it back, verify (REQ-OS-006) ----
    // Fill the scratch page with a known pattern, write one block to
    // LBA 1, zero the page, read LBA 1 back, compare. Proves the full
    // block path: data flows device-ward through the I/O SQ and back.
    const WRT_MAGIC: &[u8] = b"OMNIOSK-WRT-OK!";
    for (i, b) in WRT_MAGIC.iter().enumerate() {
        wbyte(readb, i, *b);
    }
    let mut sqe = [0u8; 64];
    sqe[0] = 0x01; // Write
    sqe[2..4].copy_from_slice(&5u16.to_le_bytes()); // CID 5
    sqe[4..8].copy_from_slice(&1u32.to_le_bytes()); // NSID 1
    sqe[24..32].copy_from_slice(&(readb as u64).to_le_bytes()); // PRP1
    sqe[40..44].copy_from_slice(&1u32.to_le_bytes()); // CDW10: SLBA lo = 1
    sqe[48..52].copy_from_slice(&0u32.to_le_bytes()); // CDW12: NLB-1 = 0
    submit(bar, iosq, 1, sq_db_off(1, stride), &sqe);
    let dw3 = match wait_cqe(bar, iocq, 1, 1, cq_hd_off(1, stride), IO_QSIZE) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: write timeout\n");
            return None;
        }
    };
    if (dw3 >> 17 & 0x7FFF) != 0 {
        uart.write_str("NVME: write status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return None;
    }

    // Zero the scratch page, then read LBA 1 back and compare.
    for i in 0..WRT_MAGIC.len() {
        wbyte(readb, i, 0);
    }
    let mut sqe = [0u8; 64];
    sqe[0] = 0x02; // Read
    sqe[2..4].copy_from_slice(&6u16.to_le_bytes()); // CID 6
    sqe[4..8].copy_from_slice(&1u32.to_le_bytes()); // NSID 1
    sqe[24..32].copy_from_slice(&(readb as u64).to_le_bytes()); // PRP1
    sqe[40..44].copy_from_slice(&1u32.to_le_bytes()); // CDW10: SLBA lo = 1
    sqe[48..52].copy_from_slice(&0u32.to_le_bytes()); // CDW12: NLB-1 = 0
    submit(bar, iosq, 2, sq_db_off(1, stride), &sqe);
    let dw3 = match wait_cqe(bar, iocq, 2, 1, cq_hd_off(1, stride), IO_QSIZE) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: write-verify read timeout\n");
            return None;
        }
    };
    if (dw3 >> 17 & 0x7FFF) != 0 {
        uart.write_str("NVME: write-verify status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return None;
    }
    for (i, expected) in WRT_MAGIC.iter().enumerate() {
        if vbyte(readb, i) != *expected {
            uart.write_str("NVME: write data mismatch\n");
            return None;
        }
    }
    uart.write_str("NVME WRITE PASS\n");
    Some(NvmeSession {
        bar,
        asq,
        acq,
        iosq,
        iocq,
        readb,
        lbas: nsze,
        stride,
        iosq_tail: 3,  // slots 0..2 consumed by the smoke's I/O ops
        cqe_idx: 3,
        cqe_phase: 1,
    })
}

/// Raw x86_64 byte-port I/O — scoped to this module; the single `unsafe`
/// boundary of the program is documented in main.rs (ADR-0013).
struct RawPorts;

impl PortIo for RawPorts {
    fn out(&mut self, port: u16, value: u8) {
        // SAFETY: kernel-only module; see main.rs.
        unsafe {
            core::arch::asm!("out dx, al", in("dx") port, in("al") value);
        }
    }

    fn inp(&mut self, port: u16) -> u8 {
        let value: u8;
        // SAFETY: see `out`.
        unsafe {
            core::arch::asm!("in al, dx", out("al") value, in("dx") port);
        }
        value
    }
}

impl RawPorts {
    fn out32(&mut self, port: u16, value: u32) {
        // SAFETY: kernel-only module; see main.rs.
        unsafe {
            core::arch::asm!("out dx, eax", in("dx") port, in("eax") value);
        }
    }

    fn in32(&mut self, port: u16) -> u32 {
        let value: u32;
        // SAFETY: see `out32`.
        unsafe {
            core::arch::asm!("in eax, dx", out("eax") value, in("dx") port);
        }
        value
    }
}
