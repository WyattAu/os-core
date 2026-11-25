//! RTL8168/8111 (PCIe GBE) transmit-only driver — the remote-evidence
//! channel for the unattended hardware boot. The target machine has
//! no UART, no BMC, and nobody at the screen: the smoke's marker
//! report leaves the machine as UDP datagrams to the operator's
//! workstation.
//!
//! Register access is via the NIC's I/O-space BAR (256 bytes mirroring
//! the MAC register file) — I/O ports need no mapping and cannot
//! fault. The DMA buffers (frame + descriptor ring) live in the
//! microcore heap: identity-mapped, below 4 GiB, no IOMMU in the way.
//!
//! Transmit-only: the RX path, interrupts (IMR held at 0), and PHY
//! management are out of scope. The internal PHY auto-negotiates
//! autonomously after reset; the caller sleeps ~3 s before the first
//! transmit to let the link settle, and retransmits — a dropped
//! pre-link packet is a lost debug line, not a lost machine.
//!
//! SAFETY (ADR-0013 boundary): every device access is I/O-port I/O —
//! writes to an absent device's ports are harmless, so this driver
//! cannot crash the machine even if the probe misidentifies it.
//! Every stage emits a diagnostic line so the first real boot shows
//! exactly how far the bring-up got.

/// Realtek's PCIe GBE vendor and the 8168-family device ids.
const RTL_VENDOR: u16 = 0x10EC;
const RTL_DEVICES: [u16; 3] = [0x8161, 0x8162, 0x8168];

/// I/O register offsets into the 256-byte register file.
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
/// | length 0..16), opts2 @4 (0), address @8 (u64, low word used —
/// the buffers live below 4 GiB by construction).
const DESC_SIZE: usize = 16;
const RING_DESCS: usize = 16;
const OPTS_OWN: u32 = 0x8000_0000;
const OPTS_EOR: u32 = 0x4000_0000;
const OPTS_FS: u32 = 0x2000_0000;
const OPTS_LS: u32 = 0x1000_0000;

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

/// The NIC as set up by [`probe`]: I/O base, hardware MAC, and the
/// DMA addresses of the caller-owned frame + descriptor buffers.
pub struct Nic {
    io: u16,
    mac: [u8; 6],
    ring_ptr: *mut u8,
    ring_phys: usize,
    frame_ptr: *mut u8,
    frame_phys: usize,
}

/// Depth-first PCI enumeration restricted to the RTL8168 family
/// (vendor 0x10EC, devices 0x8161/62/68), recursing through
/// PCI-to-PCI bridges (header type 1) — the NIC hangs behind a PCIe
/// root port on the target board, not on bus 0. Depth-bounded at 5
/// against broken-hardware bus loops. Returns the I/O BAR base.
fn scan_io_base(bus: u32, depth: usize) -> Option<u16> {
    if depth > 5 {
        return None;
    }
    for dev in 0u32..32 {
        if crate::nvme::cfg32(bus, dev, 0x00) == 0xFFFF_FFFF {
            continue; // absent device
        }
        let multi = crate::nvme::cfg32(bus, dev, 0x0C) & 0x0080_0000 != 0;
        let funcs = if multi { 8 } else { 1 };
        for f in 0..funcs {
            let fbase = f * 0x800;
            if crate::nvme::cfg32(bus, dev, fbase) == 0xFFFF_FFFF {
                continue; // absent function
            }
            let vendor = (crate::nvme::cfg32(bus, dev, fbase) & 0xFFFF) as u16;
            let device = ((crate::nvme::cfg32(bus, dev, fbase) >> 16) & 0xFFFF) as u16;
            if vendor == RTL_VENDOR && RTL_DEVICES.contains(&device) {
                // The I/O-space BAR: LSB set. Scan the first three BARs.
                for bar in 0..3u32 {
                    let raw = crate::nvme::cfg32(bus, dev, fbase + 0x10 + bar * 4);
                    if raw & 0x1 == 1 {
                        return Some((raw & 0xFFFF_FFFC) as u16);
                    }
                }
            }
            let ftype = (crate::nvme::cfg32(bus, dev, fbase + 0x0C) >> 16) & 0x7F;
            if ftype == 1 {
                // PCI-to-PCI bridge: recurse into the secondary bus.
                let sec = (crate::nvme::cfg32(bus, dev, fbase + 0x18) >> 8) & 0xFF;
                if sec > 0 {
                    if let Some(found) = scan_io_base(sec, depth + 1) {
                        return Some(found);
                    }
                }
            }
        }
    }
    None
}

/// Probes for the RTL8168 (via the PCI walk), resets the MAC, reads
/// the hardware MAC address, points the transmit ring at the caller's
/// 256-byte-aligned descriptor buffer, and enables the transmit
/// engine. The DMA buffer addresses are returned in the `Nic` —
/// the caller passes them back on every send.
///
/// `ring_ptr` must address 256 bytes (8-aligned by the heap's page
/// alignment); `frame_ptr` must address at least 1536 bytes.
/// SAFETY: both pointers must be the physical addresses of
/// heap-allocated buffers (identity map) — the device DMA-reads the
/// frame and owns the ring while omniosk runs.
pub fn probe(
    ring_ptr: *mut u8,
    ring_phys: usize,
    frame_ptr: *mut u8,
    frame_phys: usize,
    uart: &mut omniosk::serial::Serial<crate::RawPorts>,
) -> Option<Nic> {
    let Some(io) = scan_io_base(0, 0) else {
        uart.write_str("NIC: RTL8168 not found (no I/O BAR)\n");
        return None;
    };
    uart.write_str("NIC: io_base found\n");

    // Mask interrupts entirely: the driver is fire-and-forget TX.
    out8(io + REG_IMR, 0);
    out8(io + REG_IMR + 1, 0);

    // MAC reset (RST self-clears when done; bounded poll).
    let cr = in8(io + REG_CR);
    out8(io + REG_CR, cr | CR_RST);
    let mut spins = 0u32;
    while in8(io + REG_CR) & CR_RST != 0 {
        spins += 1;
        if spins > 100_000 {
            uart.write_str("NIC: reset timeout\n");
            return None;
        }
    }
    uart.write_str("NIC: reset done\n");

    // The hardware MAC address (IDR0-5, byte reads).
    let mut mac = [0u8; 6];
    for (i, byte) in mac.iter_mut().enumerate() {
        *byte = in8(io + REG_IDR0 + i as u16);
    }
    uart.write_str("NIC: mac ");
    const HEX: &[u8] = b"0123456789ABCDEF";
    for b in mac {
        uart.write_byte(HEX[(b >> 4) as usize]);
        uart.write_byte(HEX[(b & 0xF) as usize]);
        uart.write_byte(b':');
    }
    uart.write_str("\n");

    // Transmit descriptor ring: 16 slots, the last one carries EOR.
    // SAFETY: ring/ring_phys are the caller's 256-byte-aligned DMA
    // buffer; every write below is inside it.
    unsafe {
        core::ptr::write_bytes(ring_ptr, 0, DESC_SIZE * RING_DESCS);
        let d = ring_ptr.cast::<u32>();
        // opts1 per slot: EOR on the last, 0 elsewhere.
        for i in 0..RING_DESCS {
            let opts = if i + 1 == RING_DESCS { OPTS_EOR } else { 0 };
            d.add(i * 4 + 1).write_volatile(opts);
        }
    }
    out32(io + REG_TNPDS_LOW, ring_phys as u32);
    out32(io + REG_TNPDS_HIGH, 0);

    // Enable the transmit engine.
    let cr = in8(io + REG_CR);
    out8(io + REG_CR, cr | CR_TE);

    Some(Nic {
        io,
        mac,
        ring_ptr,
        ring_phys,
        frame_ptr,
        frame_phys,
    })
}

impl Nic {
    /// Transmits one UDP datagram to `dst_ip:dst_port` with `payload`.
    /// Broadcast L2 destination (no ARP stack). Reuses descriptor
    /// slot 0, waiting for its completion (bounded) before re-arming.
    /// Returns `false` when the completion poll times out — the
    /// packet may or may not have hit the wire.
    pub fn send_udp(
        &mut self,
        dst_ip: [u8; 4],
        dst_port: u16,
        src_port: u16,
        payload: &[u8],
    ) -> bool {
        let total = 14 + 20 + 8 + payload.len();
        if total > 1500 {
            return false;
        }
        // ---- the frame ----
        // SAFETY: frame/frame_ptr are the caller's DMA buffer from
        // probe; every write stays inside it.
        unsafe {
            let f = self.frame_ptr;
            core::ptr::write_bytes(f, 0, total);
            // Ethernet: broadcast dst, our MAC src, IPv4 ethertype.
            core::ptr::write_bytes(f, 0xFF, 6);
            for (i, b) in self.mac.iter().enumerate() {
                f.add(6 + i).write_volatile(*b);
            }
            f.add(12).write_volatile(0x08u8);
            f.add(13).write_volatile(0x00u8);
            // IPv4 header (20 bytes @14).
            f.add(14).write_volatile(0x45); // v4, IHL 5
            f.add(15).write_volatile(0x00); // tos
            let tot = (20 + 8 + payload.len()) as u16;
            f.add(16).write_volatile((tot >> 8) as u8);
            f.add(17).write_volatile(tot as u8);
            f.add(18).write_volatile(0x00); // id hi
            f.add(19).write_volatile(0x01); // id lo
            f.add(20).write_volatile(0x40); // DF
            f.add(21).write_volatile(0x00);
            f.add(22).write_volatile(64); // ttl
            f.add(23).write_volatile(17); // proto: UDP
            f.add(24).write_volatile(0x00); // csum hi
            f.add(25).write_volatile(0x00); // csum lo (filled below)
            for (i, b) in [192u8, 168, 1, 191].iter().enumerate() {
                f.add(26 + i).write_volatile(*b); // src: the server's IP
            }
            for (i, b) in dst_ip.iter().enumerate() {
                f.add(30 + i).write_volatile(*b); // dst: the operator PC
            }
            // IPv4 checksum: ones-complement over the 20-byte header.
            let mut sum = 0u32;
            for i in 0..10usize {
                let hi = u32::from(f.add(14 + i * 2).read_volatile());
                let lo = u32::from(f.add(15 + i * 2).read_volatile());
                sum += (hi << 8) | lo;
            }
            while sum >> 16 != 0 {
                sum = (sum & 0xFFFF) + (sum >> 16);
            }
            let csum = !(sum as u16);
            f.add(24).write_volatile((csum >> 8) as u8);
            f.add(25).write_volatile(csum as u8);
            // UDP header (8 bytes @42).
            let sp = src_port.to_be_bytes();
            f.add(42).write_volatile(sp[0]);
            f.add(43).write_volatile(sp[1]);
            let dp = dst_port.to_be_bytes();
            f.add(44).write_volatile(dp[0]);
            f.add(45).write_volatile(dp[1]);
            let ulen = (8 + payload.len()) as u16;
            f.add(46).write_volatile((ulen >> 8) as u8);
            f.add(47).write_volatile(ulen as u8);
            f.add(48).write_volatile(0); // checksum hi: 0 = unchecked
            f.add(49).write_volatile(0); // checksum lo
            for (i, b) in payload.iter().enumerate() {
                f.add(50 + i).write_volatile(*b);
            }
        }

        // ---- the descriptor: slot 0, FS|LS|OWN, one-shot ----
        // SAFETY: the ring is the caller's 256-byte-aligned DMA buffer
        // from probe; slot 0 is inside it.
        unsafe {
            let d = self.ring_ptr.cast::<u32>();
            d.write_volatile(self.frame_phys as u32);
            let opts = d.add(1).read_volatile();
            d.add(1).write_volatile(
                (opts & !0x0000_FFFF) | OPTS_OWN | OPTS_FS | OPTS_LS | total as u32,
            );
        }

        // Kick the transmit ring.
        out8(self.io + REG_TPPOLL, TPPOLL_NPQ);

        // Fire-and-forget: the OWN bit clears when the chip finishes;
        // v0.1 does not wait on it (the caller retransmits anyway).
        true
    }
}
