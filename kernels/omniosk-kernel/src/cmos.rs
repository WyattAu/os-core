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
//! bitmask u32 at 0x3A..0x3E? — both inside 0x38..0x3E (six bytes):
//! magic u16 @0x38, markers u32 @0x3A.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::PortIo;

/// The NMI-disable bit rides the 0x70 address port.
const CMOS_ADDR_NMI: u8 = 0x80;
const CMOS_ADDR_PORT: u16 = 0x70;
const CMOS_DATA_PORT: u16 = 0x71;
/// The free area this program claims.
const MARKER_MAGIC_OFF: u8 = 0x38;
const MARKER_BITS_OFF: u8 = 0x3A;
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
    CMOS_LOCK.store(false, Ordering::Relaxed);
}
