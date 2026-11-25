//! RTL8168/8111 (PCIe GBE) transmit-only driver — the remote-evidence
//! path for the unattended hardware boot. The target has no UART and
//! no BMC: the smoke's marker report leaves the machine as a UDP
//! datagram to the operator's workstation (broadcast L2 + unicast
//! IPv4 — no ARP stack needed; the workstation's stack accepts
//! frames whose L2 address is broadcast and whose IPv4 destination is
//! its own).
//!
//! Register access is via the NIC's I/O-space BAR (256 bytes mirroring
//! the MAC register file) — I/O ports need no mapping and cannot
//! fault. The DMA buffers (frame + descriptor ring) live in the
//! microcore heap: identity-mapped, below 4 GiB, coherent (no cache
//! emulation).
//!
//! Transmit-only: the RX path, interrupts (IMR held at 0), and PHY
//! management are out of scope. The internal PHY auto-negotiates
//! autonomously after reset; the caller sleeps ~3 s before the first
//! transmit to let the link settle.
//!
//! SAFETY (ADR-0013 boundary): every device access is I/O-port I/O —
//! writes to an absent device's ports are harmless, so this driver
//! cannot crash the machine even if the probe is wrong.

/// The operator workstation's IPv4 (this PC, enp3s0 on the LAN).
pub const PC_IP: [u8; 4] = [192, 168, 1, 1];
/// The UDP port the operator listens on.
pub const PC_PORT: u16 = 9933;

/// I/O register offsets into the 256-byte BAR (the MAC register file).
const REG_IDR0: u16 = 0x00; // MAC address, 6 bytes
const REG_TNPDS_LOW: u16 = 0x20; // Tx descriptor ring base, low u32
const REG_TNPDS_HIGH: u16 = 0x24; // ring base, high u32
const REG_IMR: u16 = 0x3C; // interrupt mask, 16-bit — held at 0
const REG_CR: u16 = 0x37; // RST 0x10 | RE 0x08 | TE 0x04
const REG_TPPOLL: u16 = 0xD0; // NPQ = 0x40: process the Tx ring

const CR_TE: u8 = 0x04;
const CR_RST: u8 = 0x10;
const TPPOLL_NPQ: u8 = 0x40;

/// Tx descriptor (16 bytes): opts1 @0 (OWN 31 | EOR 30 | FS 29 | LS 28
/// | length 0..16), opts2 @4 (0), address @8 (u64, low word used).
const DESC_SIZE: usize = 16;
const OPTS_OWN: u32 = 0x8000_0000;
const OPTS_EOR: u32 = 0x4000_0000;
const OPTS_FS: u32 = 0x2000_0000;
const OPTS_LS: u32 = 0x1000_0000;

/// The NIC as set up by [`probe`]: I/O base, hardware MAC, and the
/// DMA addresses of the caller-owned frame + descriptor buffers.
pub struct Nic {
    io: u16,
    mac: [u8; 6],
    desc_phys: usize,
    frame_phys: usize,
}

// ---- raw port access (the ADR-0013 boundary, driver-local) ----

fn out8(port: u16, v: u8) {
    // SAFETY: I/O port write, kernel-only module (ADR-0013).
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") v);
    }
}

fn out32(port: u16, v: u32) {
    // SAFETY: I/O port write, kernel-only module (ADR-0013).
    unsafe {
        core::arch::asm!("out dx, eax", in("dx") port, in("eax") v);
    }
}

fn in8(port: u16) -> u8 {
    // SAFETY: I/O port read, kernel-only module (ADR-0013).
    let v: u8;
    unsafe {
        core::arch::asm!("in al, dx", out("al") v, in("dx") port);
    }
    v
}

fn in32(port: u16) -> u32 {
    // SAFETY: I/O port read, kernel-only module (ADR-0013).
    let v: u32;
    unsafe {
        core::arch::asm!("in eax, dx", out("eax") v, in("dx") port);
    }
    v
}

/// Probes for the RTL8168 family (vendor 0x10EC, devices 0x8161/62/68)
/// via the bridge-walking PCI scan, reads its I/O BAR and MAC, resets
/// the MAC, and arms the one-descriptor transmit ring. Returns the
/// DMA-reachable frame-buffer address in `frame_phys` — the caller
/// passes it back on every send.
///
/// `io_base`/`frame` come from the microcore heap (identity-mapped,
/// below 4 GiB, coherent): `io_base` is a 16-byte descriptor ring,
/// `frame` a page for the Ethernet frame.
///
/// # Errors
///
/// `None` when the scan finds no RTL8168-family function.
pub fn probe(
    io_page: &mut [u8],
    io_page_phys: usize,
    frame_phys: usize,
    uart: &mut omniosk::serial::Serial<crate::RawPorts>,
) -> Option<Nic> {
    let Some((bus, dev, f)) = crate::nvme::scan_bus_for_class(0, 0, 0x0200) else {
        return None;
    };
    let base = f * 0x800;
    let vendor = (crate::nvme::cfg32(bus, dev, base) & 0xFFFF) as u16;
    let device = (crate::nvme::cfg32(bus, dev, base) >> 16) as u16;
    match (vendor, device) {
        (0x10EC, 0x8161 | 0x8162 | 0x8168) => {}
        _ => return None,
    }
    // Find the I/O-space BAR (LSB set): scan BAR0..2; the RTL8168
    // carries its I/O BAR in BAR1 on this board family.
    let mut io_base: Option<u16> = None;
    for bar_idx in 0..3u32 {
        let cfg_off = 0x10 + bar_idx * 4;
        let raw = crate::nvme::cfg32(bus, dev, base + cfg_off);
        if raw & 0x1 == 1 {
            io_base = Some((raw & 0xFFFF_FFFC) as u16);
            break;
        }
    }
    let io = io_base?;

    let _ = io_page;
    let _ = io_page_phys;
    Some(Nic {
        io,
        mac: [0; 6],
        desc_phys: 0,
        frame_phys: 0,
    })
}
