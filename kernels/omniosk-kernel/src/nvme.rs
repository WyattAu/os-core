//! Emulated NVMe access (ADR-0011 first device step).
//!
//! QEMU's `-device nvme` (Red Hat 1b36:0010) sits on PCI bus 0; SeaBIOS
//! assigned its BAR0 before the handoff, covered by the boot identity map
//! (512 GiB of 1 GiB pages). The driver speaks raw NVMe over MMIO:
//! disable controller, program the admin queues, Identify (model printed
//! over COM1), create I/O queues, read LBA 0, and verify the drive's data
//! actually round-tripped.
//!
//! Everything here is `unsafe` by package policy (ADR-0013 boundary).

use omniosk::serial::{PortIo, Serial, COM1};

const PCI_NVME_VENDOR: u16 = 0x1B36;
const PCI_NVME_DEVICE: u16 = 0x0010;

/// Page-aligned scratch: admin queues, identify data, I/O queues, read data.
#[repr(C, align(4096))]
pub struct Page {
    pub b: [u8; 4096],
}

static mut ADMIN_SQ: Page = Page { b: [0; 4096] };
static mut ADMIN_CQ: Page = Page { b: [0; 4096] };
static mut IDENT_BUF: Page = Page { b: [0; 4096] };
static mut READ_BUF: Page = Page { b: [0; 4096] };
static mut IO_SQ: Page = Page { b: [0; 4096] };
static mut IO_CQ: Page = Page { b: [0; 4096] };

const QSIZE: usize = 4;
const MAGIC: &[u8] = b"OMNIOSK-NVME-OK!";

#[inline]
fn mmio32(base: usize, off: usize) -> u32 {
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
/// doorbell (`cq_hd_off`) for the consumed entry.
fn wait_cqe(bar: usize, cq: usize, idx: usize, phase: u32, cq_hd_off: usize) -> Option<u32> {
    let mut spins = 0usize;
    let dw3 = loop {
        let v = mmio32(cq, idx * 16 + 12);
        if (v & 1) == phase {
            break v;
        }
        spins += 1;
        if spins > 50_000_000 {
            return None;
        }
    };
    mmio32w(bar, cq_hd_off, ((idx + 1) % QSIZE) as u32);
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

/// Full device bring-up + smoke. Serial carries per-stage progress and the
/// controller's model number. Returns false on any failure.
pub fn smoke() -> bool {
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
        return false;
    }

    // ---- doorbell stride from CAP.DSTRD (bits 2:0) ----
    let stride = 4u32 << (mmio32(bar, 0x00) & 0x7);
    let cq0_hd = cq_hd_off(0, stride);

    // ---- controller disable + queue programming ----
    mmio32w(bar, 0x14, 0); // CC.EN = 0
    let mut spins = 0usize;
    while mmio32(bar, 0x1C) & 1 != 0 {
        spins += 1;
        if spins > 100_000_000 {
            uart.write_str("NVME: disable timeout\n");
            return false;
        }
    }

    // SAFETY (package boundary, ADR-0013): DMA target buffers, statics.
    let asq = unsafe { core::ptr::addr_of!(ADMIN_SQ) } as usize;
    let acq = unsafe { core::ptr::addr_of!(ADMIN_CQ) } as usize;
    let ident = unsafe { core::ptr::addr_of!(IDENT_BUF) } as usize;
    let readb = unsafe { core::ptr::addr_of!(READ_BUF) } as usize;
    let iosq = unsafe { core::ptr::addr_of!(IO_SQ) } as usize;
    let iocq = unsafe { core::ptr::addr_of!(IO_CQ) } as usize;

    mmio32w(bar, 0x24, ((QSIZE as u32 - 1) << 16) | (QSIZE as u32 - 1)); // AQA
    mmio64w(bar, 0x28, asq as u64); // ASQ
    mmio64w(bar, 0x30, acq as u64); // ACQ
    mmio32w(bar, 0x14, (6 << 16) | (6 << 20) | 1); // CC: EN=1, 64B entries
    spins = 0;
    while mmio32(bar, 0x1C) & 1 == 0 {
        spins += 1;
        if spins > 100_000_000 {
            uart.write_str("NVME: enable timeout\n");
            return false;
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
    let dw3 = match wait_cqe(bar, acq, 0, 1, cq0_hd) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: identify CQE timeout\n");
            return false;
        }
    };
    if ((dw3 >> 8) & 0x7F) != 0 {
        uart.write_str("NVME: identify status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return false;
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

    // ---- I/O queues: CQ1 then SQ1 ----
    let mut sqe = [0u8; 64];
    sqe[0] = 0x05; // Create IOCQ
    sqe[2..4].copy_from_slice(&2u16.to_le_bytes()); // CID 2
    sqe[24..32].copy_from_slice(&(iocq as u64).to_le_bytes()); // PRP1
    sqe[40..44].copy_from_slice(&(QSIZE as u32 - 1).to_le_bytes()); // CDW10 size
    sqe[48..52].copy_from_slice(&1u32.to_le_bytes()); // CDW12: PC contiguous
    submit(bar, asq, 1, sq_db_off(0, stride), &sqe);
    let dw3 = match wait_cqe(bar, acq, 1, 1, cq0_hd) {
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
            return false;
        }
    };
    if ((dw3 >> 8) & 0x7F) != 0 {
        uart.write_str("NVME: IOCQ status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return false;
    }

    let mut sqe = [0u8; 64];
    sqe[0] = 0x01; // Create IOSQ
    sqe[2..4].copy_from_slice(&3u16.to_le_bytes()); // CID 3
    sqe[24..32].copy_from_slice(&(iosq as u64).to_le_bytes()); // PRP1
    sqe[40..44].copy_from_slice(&(QSIZE as u32 - 1).to_le_bytes()); // CDW10 size
    sqe[44..48].copy_from_slice(&1u32.to_le_bytes()); // CDW11: CQID 1
    sqe[48..52].copy_from_slice(&1u32.to_le_bytes()); // CDW12: PC + prio
    submit(bar, asq, 2, sq_db_off(0, stride), &sqe);
    let dw3 = match wait_cqe(bar, acq, 2, 1, cq0_hd) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: IOSQ create timeout\n");
            return false;
        }
    };
    if ((dw3 >> 8) & 0x7F) != 0 {
        uart.write_str("NVME: IOSQ status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return false;
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
    let dw3 = match wait_cqe(bar, iocq, 0, 1, cq_hd_off(1, stride)) {
        Some(v) => v,
        None => {
            uart.write_str("NVME: read timeout\n");
            return false;
        }
    };
    if ((dw3 >> 8) & 0x7F) != 0 {
        uart.write_str("NVME: read status=0x");
        write_hex(&mut uart, dw3);
        uart.write_str("\n");
        return false;
    }

    for (i, expected) in MAGIC.iter().enumerate() {
        if vbyte(readb, i) != *expected {
            uart.write_str("NVME: data mismatch\n");
            return false;
        }
    }
    true
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
