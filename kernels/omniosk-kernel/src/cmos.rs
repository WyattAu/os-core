//! CMOS NVRAM markers — the boot result's battery-backed memory.
//!
//! The hardware target has no serial port, no BMC, and (for boot #1)
//! no network driver: the only persistent evidence channel is the
//! CMOS NVRAM (ports 0x70/0x71). The NVMe smoke writes its marker
//! bitmask here at the end of a hardware run; the next boot reads it
//! back and prints it (VGA + serial). The bytes 0x38-0x3D sit in the
//! CMOS free area — modern UEFI keeps its state in NVRAM variables,
//! not CMOS, and nothing else in the program touches this range.
//!
//! Layout: magic u16 = 0x4F4D ("OM") at 0x38..0x3A, then the marker
//! bitmask u32 at 0x3A..0x3E, then a single-byte progress stage at
//! 0x3E — all inside the free area 0x38..0x3F. The stage byte is
//! written at every milestone, so a run that hangs still reports how
//! far it got (the hardware target has no serial console).

use core::sync::atomic::{AtomicBool, Ordering};

use crate::PortIo;

/// The NMI-disable bit rides the 0x70 address port.
const CMOS_ADDR_NMI: u8 = 0x80;
const CMOS_ADDR_PORT: u16 = 0x70;
const CMOS_DATA_PORT: u16 = 0x71;
/// The free area this program claims.
const MARKER_MAGIC_OFF: u8 = 0x38;
const MARKER_BITS_OFF: u8 = 0x3A;
/// Single-byte progress stage, written at every milestone.
const MARKER_STAGE_OFF: u8 = 0x3E;
const MARKER_MAGIC: u16 = 0x4F4D; // "OM"

/// Guards against concurrent CMOS access (the PIT ISR could fire mid
/// sequence — it doesn't touch CMOS, but the guard is free).
static CMOS_LOCK: AtomicBool = AtomicBool::new(false);

fn cmos_read_byte(off: u8, ports: &mut impl PortIo) -> u8 {
    ports.out(CMOS_ADDR_PORT, CMOS_ADDR_NMI | off);
    ports.inp(CMOS_DATA_PORT)
}

fn cmos_write_byte(off: u8, v: u8, ports: &mut impl PortIo) {
    ports.out(CMOS_ADDR_PORT, CMOS_ADDR_NMI | off);
    ports.out(CMOS_DATA_PORT, v);
}

/// Marker bits (bit i = the i-th hardware claim proven).
pub const M_IDENT: u32 = 1 << 0;
pub const M_READ_CONSISTENT: u32 = 1 << 1;
pub const M_SESSION_RETAINED: u32 = 1 << 2;
/// At least one UDP beacon was ACCEPTED BY THE CHIP (the descriptor's
/// OWN bit cleared). Not proof of wire delivery — a switch drop or a
/// wrong frame still counts — but it separates "the NIC never
/// transmitted" from "the datagram went out and was lost".
pub const M_UDP_OK: u32 = 1 << 3;
/// The NIC reported link-up at verdict time (PHYstatus bit 1). A run
/// with UDP missing and this bit set points at the transmit path; with
/// it clear, the PHY never negotiated during the omniosk boot.
pub const M_LINK_UP: u32 = 1 << 4;
/// Both NIC DMA buffers (ring + frame) were secured below 4 GiB. The
/// 8168 family does 32-bit DMA only — with this bit clear, a UDP miss
/// is an allocation problem, not a transmit problem.
pub const M_DMA32: u32 = 1 << 5;

/// Packs the run evidence into the ONE marker word that provably
/// survives POST on this board (0x3A-0x3D). Bits 6-7 say the MAC was
/// read this boot; bits 8-31 carry a MAC canary (XOR of the six
/// bytes, the first byte, the last byte) — for the M710q the healthy
/// values are XOR 0x59, first 0x6C, last 0x65. The wider CMOS tail
/// (0x40+) is written by POST on this AMI board and cannot hold
/// evidence.
#[must_use]
pub fn pack_markers(bits: u32, mac: [u8; 6]) -> u32 {
    let mut xor = 0u8;
    for b in mac {
        xor ^= b;
    }
    bits | (1 << 6)
        | (1 << 7)
        | (u32::from(xor) << 8)
        | (u32::from(mac[0]) << 16)
        | (u32::from(mac[5]) << 24)
}

/// Progress stages, written to `MARKER_STAGE_OFF` at each milestone.
/// Monotonic: the last byte written is the furthest stage reached, so
/// a run that hangs (no reset, no beacons) still says where it died.
pub const STAGE_ENTER: u8 = 0x01;
pub const STAGE_HEAP_OK: u8 = 0x02;
pub const STAGE_NIC_START: u8 = 0x03;
pub const STAGE_NIC_OK: u8 = 0x04;
pub const STAGE_NIC_FAIL: u8 = 0x05;
pub const STAGE_NVME_START: u8 = 0x06;
pub const STAGE_NVME_OK: u8 = 0x07;
pub const STAGE_NVME_FAIL: u8 = 0x08;
pub const STAGE_VERDICT: u8 = 0x09;
pub const STAGE_EXIT: u8 = 0x0A;
pub const STAGE_PANIC: u8 = 0xFF;

/// Writes the magic plus a progress stage. Cheap (three port writes)
/// and safe at every milestone; the magic makes a partial run
/// identifiable and the stage says where it stopped.
pub fn write_stage(ports: &mut impl PortIo, stage: u8) {
    if CMOS_LOCK
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let m = MARKER_MAGIC.to_le_bytes();
    cmos_write_byte(MARKER_MAGIC_OFF, m[0], ports);
    cmos_write_byte(MARKER_MAGIC_OFF + 1, m[1], ports);
    cmos_write_byte(MARKER_STAGE_OFF, stage, ports);
    CMOS_LOCK.store(false, Ordering::Relaxed);
}

/// Writes the marker bitmask into CMOS NVRAM. Best effort: a failed
/// lock or a dead CMOS is silently ignored (the VGA screen carries
/// the same evidence live).
pub fn write_markers(ports: &mut impl PortIo, bits: u32) {
    if CMOS_LOCK
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let m = MARKER_MAGIC.to_le_bytes();
    cmos_write_byte(MARKER_MAGIC_OFF, m[0], ports);
    cmos_write_byte(MARKER_MAGIC_OFF + 1, m[1], ports);
    let b = bits.to_le_bytes();
    for (i, byte) in b.iter().enumerate() {
        cmos_write_byte(MARKER_BITS_OFF + i as u8, *byte, ports);
    }
    CMOS_LOCK.store(false, Ordering::Relaxed);
}

/// Reads the marker bitmask; `None` when the magic is absent (no
/// prior hardware run on this machine) — i.e. the guest ran on a
/// different machine or the CMOS was cleared.
#[must_use]
pub fn read_markers(ports: &mut impl PortIo) -> Option<u32> {
    if CMOS_LOCK
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return None;
    }
    let m0 = cmos_read_byte(MARKER_MAGIC_OFF, ports);
    let m1 = cmos_read_byte(MARKER_MAGIC_OFF + 1, ports);
    let magic = u16::from_le_bytes([m0, m1]);
    if magic != MARKER_MAGIC {
        CMOS_LOCK.store(false, Ordering::Relaxed);
        return None;
    }
    let mut bits = [0u8; 4];
    for (i, byte) in bits.iter_mut().enumerate() {
        *byte = cmos_read_byte(MARKER_BITS_OFF + i as u8, ports);
    }
    CMOS_LOCK.store(false, Ordering::Relaxed);
    Some(u32::from_le_bytes(bits))
}

/// Clears the marker magic (a fresh hardware run starts clean).
pub fn clear_markers(ports: &mut impl PortIo) {
    let zero = 0u16.to_le_bytes();
    if CMOS_LOCK
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    cmos_write_byte(MARKER_MAGIC_OFF, zero[0], ports);
    cmos_write_byte(MARKER_MAGIC_OFF + 1, zero[1], ports);
    cmos_write_byte(MARKER_STAGE_OFF, 0, ports);
    CMOS_LOCK.store(false, Ordering::Relaxed);
}
