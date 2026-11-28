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

/// Register offsets into the 256-byte register file. Accessed through
/// the MMIO BAR (what Linux's r8169 uses exclusively on PCIe chips);
/// the I/O BAR is a legacy alias that real 8168 silicon does not fully
/// decode — the first hardware smoke timed out on every transmit with
/// the I/O path even though the BAR was assigned.
const REG_IDR0: u16 = 0x00; // MAC address, 6 bytes
const REG_TNPDS_LOW: u16 = 0x20; // Tx descriptor ring base, low u32
const REG_TNPDS_HIGH: u16 = 0x24; // ring base, high u32
const REG_CR: u16 = 0x37; // RST 0x10 | RE 0x08 | TE 0x04
const REG_IMR: u16 = 0x3C; // interrupt mask, 16-bit — held at 0
const REG_TXCONFIG: u16 = 0x40; // Tx DMA burst + interframe gap
const REG_RXCFG: u16 = 0x44; // Rx: FIFO thresh, DMA burst, accept bits
const REG_RMS: u16 = 0xDA; // RxMaxSize: frames larger are dropped
const REG_PHYSTATUS: u16 = 0x6C; // bit 1 = link status
const REG_RDSAR_LOW: u16 = 0xE4; // Rx ring base (latched on RE 1->0->1)
const REG_RDSAR_HIGH: u16 = 0xE8;
const REG_MPC: u16 = 0x4C; // missed packet counter (32-bit)
const REG_GPHY_OCP: u16 = 0xB8; // GPHY OCP access (flag + reg<<15 + data)
/// The transmit kick (NPQ = 0x40). On the 8168 family this is 0x38 —
/// right after ChipCmd (Linux r8169, banan-os, and Microsoft's RT
/// sample all agree). 0xD0 is the OLD RTL8169's address: our kicks
/// went there for the entire first hardware campaign and the chip
/// never saw one — reads, writes and echoes all verified while the
/// descriptor sat unfetched.
const REG_TPPOLL: u16 = 0x38;

const CR_TE: u8 = 0x04;
const CR_RE: u8 = 0x08;
const TPPOLL_NPQ: u8 = 0x40;

/// Tx descriptor (16 bytes): opts1 @0 (OWN 31 | EOR 30 | FS 29 | LS 28
/// | length 0..16), opts2 @4 (0), address @8 (u64, low word used —
/// the buffers live below 4 GiB by construction). The encoding lives
/// in `omniosk::net::encode_tx_desc` so the host tests pin the layout.
const DESC_SIZE: usize = 16;
const RING_DESCS: usize = 16;

// ---- raw bus access (the ADR-0013 boundary, driver-local) ----

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


fn mm8w(base: usize, off: u16, v: u8) {
    // SAFETY: device BAR inside the boot identity map (ADR-0013).
    unsafe { core::ptr::write_volatile((base + usize::from(off)) as *mut u8, v) }
}

fn mm32(base: usize, off: u16) -> u32 {
    // SAFETY: device BAR inside the boot identity map (ADR-0013).
    unsafe { core::ptr::read_volatile((base + usize::from(off)) as *const u32) }
}

fn mm32w(base: usize, off: u16, v: u32) {
    // SAFETY: device BAR inside the boot identity map (ADR-0013).
    unsafe { core::ptr::write_volatile((base + usize::from(off)) as *mut u32, v) }
}

/// The NIC as set up by [`probe`]: the MMIO register base (primary)
/// with the I/O BAR as a last-resort fallback, the hardware MAC, and
/// the DMA addresses of the caller-owned frame + descriptor buffers.
pub struct Nic {
    /// MMIO BAR base (0 = absent, use the I/O BAR).
    mem: usize,
    io: u16,
    mac: [u8; 6],
    ring_ptr: *mut u8,
    ring_phys: usize,
    frame_ptr: *mut u8,
    frame_phys: usize,
    /// Command-register readback after the bus-master write (echo:
    /// bits 0-2 must be IO|MEM|BusMaster, want 0x07 — without BM the
    /// chip cannot start the descriptor DMA at all).
    cmd_echo: u8,
    /// The next ring slot to arm. The chip walks the ring from TNPDS
    /// and stops at the first descriptor it does not own, so each send
    /// arms the next slot; slot 15 carries EOR and wraps to slot 0.
    tx_slot: usize,
    /// Receive ring: descriptor array (8 x 16 B) and the physical
    /// base of each descriptor's 4 KiB buffer. Poll-only: no Rx
    /// interrupts, the kernel task walks the ring every tick.
    rx_ring: *mut u8,
    rx_buf_phys: [usize; 8],
    rx_tail: usize,
}

// SAFETY: the Nic lives in the boot thread until the verdict, then is
// handed exclusively to the PIT-dispatched task on the same core (ISR
// interleaving, never parallel access); its raw pointers target
// identity-mapped RAM that is never freed while the kernel runs.
unsafe impl Send for Nic {}
unsafe impl Sync for Nic {}

/// Rx ring shape: 8 descriptors x 4 KiB buffers, feed frames are
/// well under the 2048-byte RxMaxSize.
const RX_RING_DESCS: usize = 8;
const RX_BUF_SIZE: u32 = 4096;

impl Nic {
    /// The 8168's MMIO register file is only reliably readable with
    /// dword accesses: byte reads returned zeros for the MAC (IDR0-5)
    /// on the M710q while dword reads work — so every byte read is
    /// derived from an aligned dword read.
    fn r8(&self, off: u16) -> u8 {
        if self.mem != 0 {
            let base = self.mem + usize::from(off & 0xFFFC);
            let shift = (off & 3) * 8;
            // SAFETY: device BAR inside the boot identity map (ADR-0013).
            ((unsafe { core::ptr::read_volatile(base as *const u32) }) >> shift) as u8
        } else {
            in8(self.io + off)
        }
    }

    fn r32(&self, off: u16) -> u32 {
        if self.mem != 0 {
            mm32(self.mem, off)
        } else {
            u32::from(in8(self.io + off)) | (u32::from(in8(self.io + off + 1)) << 8)
        }
    }

    fn w8(&self, off: u16, v: u8) {
        // Writes go through the I/O BAR when present: port I/O is the
        // proven write path on this platform (CMOS, PIC, PIT all use
        // it), while posted MMIO writes to this endpoint were never
        // verified to land. Reads stay on MMIO (the MAC canary proves
        // them). The I/O BAR mirrors the same 256-byte register file.
        if self.io != 0 {
            out8(self.io + off, v);
        } else {
            mm8w(self.mem, off, v);
        }
    }

    fn w32(&self, off: u16, v: u32) {
        if self.io != 0 {
            out32(self.io + off, v);
        } else {
            mm32w(self.mem, off, v);
        }
    }

    /// PHYstatus (0x6C) bit 1: the link is up (verified against the
    /// banan-os RTL8169 `PHYSts` enum: FullDup=bit0, LinkSts=bit1).
    #[must_use]
    pub fn link_up(&self) -> bool {
        self.r8(REG_PHYSTATUS) & 0x02 != 0
    }

    /// The MAC the driver read from IDR0-5 (diagnostic canary).
    #[must_use]
    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    /// TxConfig byte 1 echo: bits 15:8 carry the TX DMA-burst config;
    /// the healthy driver value echoes 0x0f.
    #[must_use]
    pub fn txcfg_b1(&self) -> u8 {
        (self.r32(REG_TXCONFIG) >> 8) as u8
    }

    /// ChipCmd readback (echo: RE|TE must be set after rx_start).
    #[must_use]
    pub fn chipcmd(&self) -> u8 {
        self.r8(REG_CR)
    }

    /// RxConfig byte 0 echo: the accept bits (AB|AMC|ACP want 0x0E).
    #[must_use]
    pub fn rxcfg_echo(&self) -> u8 {
        self.r32(REG_RXCFG) as u8
    }

    /// Missed-packet counter low byte (register 0x4C, count in bits
    /// 23:0): frames the chip received but could not deposit — > 0
    /// means the ring or RDSAR is the problem, 0 means nothing was
    /// received at all.
    #[must_use]
    pub fn missed_lo(&self) -> u8 {
        self.r32(REG_MPC) as u8
    }

    /// GPHY OCP read: register window 0xB8, OCPAR flag 0x8000_0000,
    /// `reg << 15`. Returns the 16-bit PHY register value once the
    /// flag sets, None on timeout.
    fn gphy_ocp_read(&self, reg: u32) -> Option<u16> {
        if reg > 0xFFFF {
            return None;
        }
        self.w32(REG_GPHY_OCP, reg << 15);
        let mut spins = 0u32;
        while spins < 4_000 {
            let v = self.r32(REG_GPHY_OCP);
            if v & 0x8000_0000 != 0 {
                return Some((v & 0xFFFF) as u16);
            }
            spins += 1;
            core::hint::spin_loop();
        }
        None
    }

    /// GPHY OCP write: flag + `reg << 15` + data; the flag clears on
    /// completion. Returns false on timeout.
    fn gphy_ocp_write(&self, reg: u32, data: u16) -> bool {
        if reg > 0xFFFF {
            return false;
        }
        self.w32(REG_GPHY_OCP, 0x8000_0000 | (reg << 15) | u32::from(data));
        let mut spins = 0u32;
        while spins < 4_000 {
            if self.r32(REG_GPHY_OCP) & 0x8000_0000 == 0 {
                return true;
            }
            spins += 1;
            core::hint::spin_loop();
        }
        false
    }

    /// Disables the RTL8168G PHY ALDPS (Advanced Link Power Down
    /// Power Saving): Linux rtl8168g_disable_aldps — clear BIT(2) of
    /// PHY paged register 0x0a43:0x10. With firmware's ALDPS left on,
    /// the PHY receiver power-gates and the MAC sees no frames at all
    /// (missed-packet counter stays 0) despite a trained link and Tx
    /// working — the exact RX-death signature of the M710q runs.
    /// GPHY OCP address: page 0x0a43 << 4 + reg 0x10 * 2 = 0xa450.
    pub fn phy_aldps_off(&mut self) -> bool {
        let addr = 0xA450;
        let Some(v) = self.gphy_ocp_read(addr) else {
            return false;
        };
        if v & 0x04 == 0 {
            return true; // already disabled
        }
        self.gphy_ocp_write(addr, v & !0x04)
    }

    /// The command-register readback after the bus-master write
    /// (echo: bits 0-2 = IO|MEM|BusMaster, want 0x07).
    #[must_use]
    pub fn cmd_echo(&self) -> u8 {
        self.cmd_echo
    }

    /// Hex-encodes the state dump into `out` (`"xxxx"` per dword).
    /// Returns bytes written.
    pub fn hex_state(&self, out: &mut [u8]) -> usize {
        let (st, _gphy_ok) = self.state_dump();
        const HEX: &[u8] = b"0123456789ABCDEF";
        let mut w = 0usize;
        for v in st {
            for shift in [28u32, 24, 20, 16, 12, 8, 4, 0] {
                if w + 1 < out.len() {
                    out[w] = HEX[usize::from(((v >> shift) & 0xF) as u8)];
                    w += 1;
                }
            }
        }
        w
    }

    /// Full-chip state dump for the beacon diagnostic channel: every
    /// register and descriptor word that could explain a silent RX
    /// path, in one payload. `gphy_ok` false means the PHY OCP window
    /// timed out (PHY config reads are then garbage).
    #[must_use]
    pub fn state_dump(&self) -> ([u32; 16], bool) {
        let mut st = [0u32; 16];
        st[0] = self.r32(REG_TXCONFIG);
        st[1] = self.r32(REG_RXCFG);
        st[2] = self.r32(REG_TNPDS_LOW);
        st[3] = self.r32(REG_RDSAR_LOW);
        st[4] = self.r32(REG_RMS);
        st[5] = self.r32(REG_MPC);
        st[6] = u32::from(self.r8(REG_CR));
        st[7] = self.r32(0x3C); // IMR (lo) + ISR (hi)
        let al = self.gphy_ocp_read(0xA450); // ALDPS paged reg
        st[8] = al.map(u32::from).unwrap_or(0xFFFF_FFFF);
        st[9] = self.gphy_ocp_read(0xA400 + 2).map(u32::from).unwrap_or(0xFFFF_FFFF); // BMCR
        st[10] = self
            .gphy_ocp_read(0xA400 + 4)
            .map(u32::from)
            .unwrap_or(0xFFFF_FFFF); // BMSR (link)
        st[11] = self.r32(0x38); // TxPoll (should read NPQ echo)
        // live Rx descriptors 0-2: OWN state says whether the chip
        // ever touched our ring.
        for i in 0..3 {
            // SAFETY: tail-independent ring read; the page is ours and
            // identity-mapped.
            let o1 =
                unsafe { core::ptr::read_volatile(self.rx_ring.add(i * DESC_SIZE).cast::<u32>()) };
            match omniosk::net::decode_rx_done(o1) {
                Some((len, f, l)) => {
                    st[12 + i] = (len as u32)
                        | if f { 1 << 29 } else { 0 }
                        | if l { 1 << 28 } else { 0 };
                }
                None => {
                    st[12 + i] = 0xFFFF_FFFF; // still owned by chip
                }
            }
        }
        (st, st[8] != 0xFFFF_FFFF)
    }

    /// True when the chip's TNPDS echo equals our ring base in full —
    /// i.e. the base it walks is the one we programmed. The high byte
    /// alone is ambiguous: the heap sits below 16 MiB on this boot, so
    /// a default-0 register echoes 0 spuriously.
    #[must_use]
    pub fn ring_latched(&self) -> bool {
        self.r32(REG_TNPDS_LOW) == self.ring_phys as u32
    }

    /// TNPDS echo, byte 2 (bits 23:16 of the ring base). The heap's
    /// conservative layout puts the ring at 0x01-0x02 MiB-region, so a
    /// landed write echoes 0x01-0x02 here and a default reads 0x00.
    #[must_use]
    pub fn ring_mid(&self) -> u8 {
        (self.r32(REG_TNPDS_LOW) >> 16) as u8
    }

    /// RDSAR echo, byte 2 (bits 23:16 of the RX ring base): 0x12 when
    /// our ring latched, 0x00 when the write is unlanded or the
    /// register is write-only on this silicon.
    #[must_use]
    pub fn rdsar_mid(&self) -> u8 {
        (self.r32(REG_RDSAR_LOW) >> 16) as u8
    }

    /// Brings up the receive ring: 8 descriptors, one 4 KiB buffer
    /// each, poll-only (IMR stays masked; the kernel task walks the
    /// ring). RxMaxSize 4096 matches the buffers; RxConfig gets the
    /// Linux VER_40 value plus accept-phys/broadcast/multicast. The
    /// ring base is programmed across a RE toggle, mirroring the TNPDS
    /// latch (the chip takes the base on the 1->0->1 transition).
    ///
    /// `ring` must address 128 writable bytes (one heap page);
    /// `buf_phys` holds the physical address of one 4 KiB page per
    /// descriptor.
    pub fn rx_start(&mut self, ring: *mut u8, ring_phys: usize, buf_phys: &[usize; 8]) {
        self.rx_ring = ring;
        self.rx_buf_phys = *buf_phys;
        self.rx_tail = 0;
        // SAFETY: the ring page is ours, identity-mapped; every write
        // stays inside the 128-byte descriptor array.
        unsafe {
            core::ptr::write_bytes(ring, 0, RX_RING_DESCS * DESC_SIZE);
            for (i, bp) in buf_phys.iter().enumerate() {
                let desc = omniosk::net::encode_rx_desc(
                    *bp as u64,
                    RX_BUF_SIZE,
                    i + 1 == RX_RING_DESCS,
                );
                core::ptr::copy_nonoverlapping(
                    desc.as_ptr(),
                    ring.add(i * DESC_SIZE),
                    DESC_SIZE,
                );
            }
        }
        self.w32(REG_RDSAR_LOW, ring_phys as u32);
        self.w32(REG_RDSAR_HIGH, (ring_phys >> 32) as u32);
        // RxMaxSize: frames larger than a buffer are dropped by the
        // chip; 4096 matches the buffers (word register, two bytes).
        self.w8(REG_RMS, (RX_BUF_SIZE & 0xFF) as u8);
        self.w8(REG_RMS + 1, (RX_BUF_SIZE >> 8) as u8);
        // RxConfig: PRESERVE the firmware-programmed value and only OR
        // in the accept bits (phys/multicast/broadcast). The first
        // attempt OVERWROTE the register with the Linux VER_40 default
        // (0xe70e) — that clobbered G-generation enable bits firmware
        // had set (bits 14-17) and the MAC went deaf: RxCfg echoed
        // 0x0e, ChipCmd 0x0c, missed counter 0, zero frames deposited
        // across a 5-minute broadcast storm. OR-in only.
        let cfg = self.r32(REG_RXCFG);
        self.w32(REG_RXCFG, cfg | 0x0E);
        let cr = self.r8(REG_CR);
        self.w8(REG_CR, cr & !CR_RE);
        self.w8(REG_CR, cr | CR_RE | CR_TE);
        // The PHY receiver must be un-power-gated for any of the above
        // to matter (see phy_aldps_off).
        self.phy_aldps_off();
    }

    /// Polls one received frame into `out` and re-arms the descriptor.
    /// Returns the frame length (Ethernet header + payload, FCS
    /// stripped) or `None` when the chip still owns the descriptor
    /// (nothing new) or the frame was fragmented/oversized (re-armed
    /// and skipped).
    pub fn rx_next(&mut self, out: &mut [u8]) -> Option<usize> {
        let i = self.rx_tail;
        // SAFETY: tail < RX_RING_DESCS; the ring page is ours and
        // identity-mapped.
        let desc = unsafe { self.rx_ring.add(i * DESC_SIZE) };
        // SAFETY: the ring page is ours, identity-mapped; the volatile
        // read observes the chip's DMA completion write (coherent).
        let opts1 = unsafe { core::ptr::read_volatile(desc.cast::<u32>()) };
        let Some((len_incl, first, last)) = omniosk::net::decode_rx_done(opts1) else {
            return None;
        };
        let mut got = None;
        // A complete, fitting frame: hand it out (FCS stripped).
        if first && last && len_incl >= 4 && len_incl - 4 <= out.len() {
            let n = len_incl - 4;
            let buf = self.rx_buf_phys[i];
            // SAFETY: the buffer page is ours, identity-mapped; the
            // length is bounded by RxMaxSize <= the buffer size.
            let frame =
                unsafe { core::slice::from_raw_parts(buf as *const u8, n) };
            out[..n].copy_from_slice(frame);
            got = Some(n);
        }
        // Re-arm the descriptor for the chip (same buffer, OWN again).
        // SAFETY: same ring bounds as the read above.
        let rearm = omniosk::net::encode_rx_desc(
            self.rx_buf_phys[i] as u64,
            RX_BUF_SIZE,
            i + 1 == RX_RING_DESCS,
        );
        unsafe {
            core::ptr::copy_nonoverlapping(rearm.as_ptr(), desc, DESC_SIZE);
        }
        self.rx_tail = (i + 1) % RX_RING_DESCS;
        got
    }
}

/// Writes one config dword (the CONFIG_ADDRESS/CFC mechanism, same
/// addressing nvme::cfg32 reads).
fn cfg32w(bus: u32, dev: u32, off: u32, v: u32) {
    let addr = 0x8000_0000 | (bus << 16) | ((dev & 0x1F) << 11) | (off & 0xFC);
    out32(0xCF8, addr);
    out32(0xCFC, v);
}

/// Walks the PCI capability list and returns the offset of the given
/// capability ID, or None.
fn cap_off(bus: u32, dev: u32, fbase: u32, want: u32) -> Option<u16> {
    let mut p = (crate::nvme::cfg32(bus, dev, fbase + 0x34) & 0xFF) as u16;
    let mut hops = 0;
    while p != 0 && p != 0xFF && hops < 16 {
        let cap = crate::nvme::cfg32(bus, dev, fbase + u32::from(p));
        if cap & 0xFF == want {
            return Some(p);
        }
        p = ((cap >> 8) & 0xFF) as u16;
        hops += 1;
    }
    None
}

/// Walks the PCI capability list and returns the PCIe capability
/// (ID 0x10) offset, or None.
fn pcie_cap_off(bus: u32, dev: u32, fbase: u32) -> Option<u16> {
    cap_off(bus, dev, fbase, 0x10)
}

/// Clears the ASPM control bits (LnkCtl[1:0]) on a PCIe function.
/// Firmware owns ASPM on this board ("can't disable ASPM; OS doesn't
/// have ASPM control" — the r8169 driver's own log line) and leaves the
/// link power-managed: non-posted reads wake the link and succeed,
/// while posted writes are silently dropped — registers read fine but
/// our configuration and the transmit kick never landed. Bare metal
/// simply clears the two bits in both endpoints of the link.
fn aspm_disable(bus: u32, dev: u32, fbase: u32, uart: &mut omniosk::serial::Serial<crate::RawPorts>) {
    let Some(cap) = pcie_cap_off(bus, dev, fbase) else {
        return;
    };
    let lnkctl = fbase + u32::from(cap) + 0x10;
    let raw = crate::nvme::cfg32(bus, dev, lnkctl);
    if raw & 0x3 != 0 {
        cfg32w(bus, dev, lnkctl, raw & 0xFFFF_FFFC);
        let after = crate::nvme::cfg32(bus, dev, lnkctl);
        uart.write_str(if after & 0x3 == 0 {
            "NIC: ASPM disabled\n"
        } else {
            "NIC: ASPM clear did not stick\n"
        });
    }
}

/// Depth-first PCI enumeration restricted to the RTL8168 family
/// (vendor 0x10EC, devices 0x8161/62/68), recursing through
/// PCI-to-PCI bridges (header type 1) — the NIC hangs behind a PCIe
/// root port on the target board, not on bus 0. Depth-bounded at 5
/// against broken-hardware bus loops. Returns (MMIO BAR base, I/O BAR
/// base, device location); the MMIO BAR is the register path that
/// works on real 8168 silicon, the I/O BAR is the legacy fallback.
fn scan_bars(
    bus: u32,
    depth: usize,
    bridge: Option<(u32, u32, u32)>,
) -> (
    Option<usize>,
    Option<u16>,
    Option<(u32, u32, u32)>,
    Option<(u32, u32, u32)>,
) {
    if depth > 5 {
        return (None, None, None, None);
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
                let mut mem = None;
                let mut io = None;
                let mut bar = 0u32;
                while bar < 4 {
                    let raw = crate::nvme::cfg32(bus, dev, fbase + 0x10 + bar * 4);
                    if raw == 0xFFFF_FFFF {
                        bar += 1;
                        continue;
                    }
                    if raw & 0x1 == 1 {
                        if io.is_none() {
                            io = Some((raw & 0xFFFF_FFFC) as u16);
                        }
                        bar += 1;
                    } else {
                        // Memory BAR: 64-bit BARs occupy two slots.
                        let wide = raw & 0x6 == 0x4;
                        if mem.is_none() {
                            let mut base = (raw & 0xFFFF_FFF0) as u64;
                            if wide {
                                let hi =
                                    crate::nvme::cfg32(bus, dev, fbase + 0x10 + (bar + 1) * 4);
                                base |= (u64::from(hi)) << 32;
                            }
                            // A BAR above the identity map cannot be
                            // touched; keep scanning for a usable one.
                            if base != 0 && base < 0x0000_8000_0000_0000 {
                                mem = Some(base as usize);
                            }
                        }
                        bar += usize::from(wide) as u32 + 1;
                    }
                }
                return (mem, io, Some((bus, dev, fbase)), bridge);
            }
            let ftype = (crate::nvme::cfg32(bus, dev, fbase + 0x0C) >> 16) & 0x7F;
            if ftype == 1 {
                // PCI-to-PCI bridge: recurse into the secondary bus.
                let sec = (crate::nvme::cfg32(bus, dev, fbase + 0x18) >> 8) & 0xFF;
                if sec > 0 {
                    let this_bridge = Some((bus, dev, fbase));
                    let (mem, io, loc, br) =
                        scan_bars(sec, depth + 1, bridge.or(this_bridge));
                    if mem.is_some() || io.is_some() {
                        return (mem, io, loc, br);
                    }
                }
            }
        }
    }
    (None, None, None, None)
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
    let (mem, io, loc, bridge) = scan_bars(0, 0, None);
    if mem.is_none() && io.is_none() {
        uart.write_str("NIC: RTL8168 not found (no BARs)\n");
        return None;
    }
    let mem = mem.unwrap_or(0);
    let io = io.unwrap_or(0);
    if mem != 0 {
        uart.write_str("NIC: mmio_base found\n");
    } else {
        uart.write_str("NIC: io_base found (MMIO BAR absent)\n");
    }

    // ASPM off on BOTH ends of the link before any MMIO access (see
    // `aspm_disable`), and enable the root port itself (IO+MEM+Bus
    // master): the NVMe works behind ITS port because firmware booted
    // from it; nobody ever turned on the NIC's port for forwarding.
    for target in [loc, bridge].into_iter().flatten() {
        let (b, d, fb) = target;
        aspm_disable(b, d, fb, uart);
    }
    if let Some((bb, bd, bfb)) = bridge {
        let raw = crate::nvme::cfg32(bb, bd, bfb + 0x04);
        cfg32w(bb, bd, bfb + 0x04, (raw & 0xFFFF_0000) | 0x0007);
        uart.write_str("NIC: root port enabled (io+mem+busmaster)\n");
    }

    // Power state D0: if firmware parked the NIC in D3 (it was never
    // boot-critical), config still answers but the register file reads
    // zeros and drops writes — exactly the observed window behavior.
    if let Some((b, d, fb)) = loc {
        if let Some(pm) = cap_off(b, d, fb, 0x01) {
            let pmcsr = fb + u32::from(pm) + 4;
            let raw = crate::nvme::cfg32(b, d, pmcsr);
            if raw & 0x3 != 0 {
                cfg32w(b, d, pmcsr, raw & 0xFFFF_FFFC);
                uart.write_str("NIC: PMCSR -> D0\n");
            }
        }
    }

    // Bus master + I/O + memory decode. The NVMe works without this
    // write only because firmware used it to boot; the NIC was never
    // boot-critical, so its command register arrived with BusMaster
    // clear — registers read fine, the PHY ran, but the DMA engine
    // never fetched a descriptor. Keep the status half of the dword:
    // the command register is the low 16 bits only. The readback is
    // the echo canary: if the config DATA write does not land, Bus
    // Master stays off and the chip cannot DMA at all.
    let mut cmd_echo = 0u8;
    if let Some((b, d, fb)) = loc {
        let raw = crate::nvme::cfg32(b, d, fb + 0x04);
        cfg32w(b, d, fb + 0x04, (raw & 0xFFFF_0000) | 0x0007);
        cmd_echo = (crate::nvme::cfg32(b, d, fb + 0x04) & 0xFF) as u8;
        uart.write_str("NIC: cmd echo ");
        const HEXC: &[u8] = b"0123456789ABCDEF";
        uart.write_byte(HEXC[usize::from(cmd_echo >> 4)]);
        uart.write_byte(HEXC[usize::from(cmd_echo & 0xF)]);
        uart.write_byte(b'\n');
    }

    // ---- the fingerprint dump ----
    // REMOVED: the wide CMOS tail (0x40+) is written by POST on this
    // AMI board and cannot hold evidence; the diagnostic content moved
    // into `pack_markers` (MAC canary in the proven 0x3A-0x3D word).

    // NO soft reset. The RTL8168G zeroes its register file on the RST
    // bit — including the autoloaded MAC registers, which never re-load
    // until a PCI-level reset — and stayed half-dead for the whole run:
    // MAC read 00:00:00:00:00:00, TxConfig echoed 0, no descriptor ever
    // taken (the CMOS canaries pinned this across three boots).
    // Firmware hands the chip over ALIVE — MAC loaded, Rx enabled,
    // link up — so reuse that state: keep ChipCmd, just make sure the
    // transmit engine is on.
    let mut nic = Nic {
        mem,
        io,
        mac: [0; 6],
        ring_ptr,
        ring_phys,
        frame_ptr,
        frame_phys,
        cmd_echo,
        tx_slot: 0,
        rx_ring: core::ptr::null_mut(),
        rx_buf_phys: [0; 8],
        rx_tail: 0,
    };
    // Interrupts stay masked (fire-and-forget TX); a dropped IMR write
    // is harmless — firmware's IMR may still line up IRQs, but the
    // kernel's IDT only owns vector 0x50 and the fault gates.
    nic.w8(REG_IMR, 0);
    nic.w8(REG_IMR + 1, 0);
    let cr = nic.r8(REG_CR);
    nic.w8(REG_CR, cr | CR_TE);
    uart.write_str("NIC: transmit enabled (no soft reset)\n");

    // TxConfig: the DMA-burst/interframe-gate configuration. Firmware's
    // hand-off value is unknown; the healthy driver runs 0x4f000f80 on
    // this chip and transmits with it. Without a sane value here the
    // chip owns the ring but never DMAs a descriptor (the TE echo, the
    // TNPDS latch and the kick were all verified good while UDP_OK
    // stayed clear).
    const TXCFG_HEALTHY: u32 = 0x4F00_0F80;
    nic.w32(REG_TXCONFIG, TXCFG_HEALTHY);

    // Clock-request / ASPM-L1 gating at the chip level: Config2.7 and
    // Config5.0. Linux's rtl_hw_start does this FIRST
    // (rtl_hw_aspm_clkreq_enable(false)) — with firmware's power bits
    // left on, the register file answers and writes echo, but the DMA
    // engine's clock domain stays gated and descriptor fetches never
    // start (everything else verified good while UDP_OK stayed clear).
    // The Config registers are LOCKED: writes land only while the
    // 9346 magic is unlocked (rtl_unlock_config_regs writes 0xC0).
    const REG_CONFIG2: u16 = 0x53;
    const REG_CONFIG5: u16 = 0x56;
    const REG_CFG9346: u16 = 0x50;
    nic.w8(REG_CFG9346, 0xC0); // unlock config registers
    let c2 = nic.r8(REG_CONFIG2);
    nic.w8(REG_CONFIG2, c2 & !0x80); // ClkReqEn off
    let c5 = nic.r8(REG_CONFIG5);
    nic.w8(REG_CONFIG5, c5 & !0x01); // ASPM_en off
    nic.w8(REG_CFG9346, 0x00); // lock again
    uart.write_str("NIC: clkreq/aspm-l1 cleared (config unlocked)\n");

    // The hardware MAC address: two aligned dword reads (IDR0-3,
    // IDR4-5 in the low half of the second). Byte reads of IDR returned
    // all zeros on real 8168 silicon.
    let mut mac = [0u8; 6];
    if mem != 0 {
        let lo = nic.r32(REG_IDR0);
        let hi = nic.r32(REG_IDR0 + 4);
        mac[0] = lo as u8;
        mac[1] = (lo >> 8) as u8;
        mac[2] = (lo >> 16) as u8;
        mac[3] = (lo >> 24) as u8;
        mac[4] = hi as u8;
        mac[5] = (hi >> 8) as u8;
    } else {
        for (i, byte) in mac.iter_mut().enumerate() {
            *byte = nic.r8(REG_IDR0 + i as u16);
        }
    }
    nic.mac = mac;
    uart.write_str("NIC: mac ");
    const HEX: &[u8] = b"0123456789ABCDEF";
    for b in mac {
        uart.write_byte(HEX[(b >> 4) as usize]);
        uart.write_byte(HEX[(b & 0xF) as usize]);
        uart.write_byte(b':');
    }
    uart.write_str(if nic.link_up() {
        " link UP\n"
    } else {
        " link down\n"
    });

    // Transmit descriptor ring: 16 slots, zeroed. The encoder sets
    // OWN and (on the last slot) EOR; an unowned slot stops the chip.
    // SAFETY: ring/ring_phys are the caller's 256-byte-aligned DMA
    // buffer; every write below is inside it.
    unsafe {
        core::ptr::write_bytes(ring_ptr, 0, DESC_SIZE * RING_DESCS);
    }
    // The chip LATCHES the ring base on the TE 1->0->1 transition.
    // Firmware hands TE already enabled, so writing TNPDS mid-enable
    // left the chip polling ITS OWN old ring address — our descriptors
    // were never read (OWN never cleared, zero beacons). Toggle TE off,
    // program the ring, re-enable.
    let cr = nic.r8(REG_CR);
    nic.w8(REG_CR, cr & !CR_TE);
    nic.w32(REG_TNPDS_LOW, ring_phys as u32);
    nic.w32(REG_TNPDS_HIGH, 0);
    nic.w8(REG_CR, cr | CR_TE);

    Some(nic)
}

impl Nic {
    /// Transmits one UDP datagram to `dst_ip:dst_port` with `payload`.
    /// The L2 destination is broadcast (no ARP stack). Each send arms
    /// the next ring slot and waits (bounded) for the chip to release
    /// it, so the shared frame buffer is safe to reuse.
    ///
    /// Returns `false` when the payload is too large or the chip never
    /// releases the slot — the packet may or may not have hit the wire.
    pub fn send_udp(
        &mut self,
        dst_ip: [u8; 4],
        dst_port: u16,
        src_port: u16,
        payload: &[u8],
    ) -> bool {
        // SAFETY: frame_ptr is the caller's DMA buffer from probe —
        // identity-mapped and at least MAX_FRAME bytes.
        let frame = unsafe {
            core::slice::from_raw_parts_mut(self.frame_ptr, omniosk::net::MAX_FRAME)
        };
        let Some(total) = omniosk::net::build_udp_frame(
            frame,
            self.mac,
            [192, 168, 1, 191], // the server's fixed LAN address
            dst_ip,
            src_port,
            dst_port,
            payload,
        ) else {
            return false;
        };

        // ---- the descriptor: rolling slot, RTL8168 layout ----
        // opts1 @0 (OWN|EOR|FS|LS|len), opts2 @4, addr @8 (u64).
        // The previous revision wrote the address into opts1 and the
        // flags into opts2: the chip never saw OWN and transmitted
        // nothing (the hardware smoke's zero-beacon failure).
        let slot = self.tx_slot;
        let eor = slot + 1 == RING_DESCS;
        let desc = omniosk::net::encode_tx_desc(self.frame_phys as u64, total as u16, eor);
        let base = slot * DESC_SIZE;

        // SAFETY: `slot < RING_DESCS` and the ring is the caller's
        // 256-byte-aligned DMA buffer from probe.
        unsafe {
            let d = self.ring_ptr.add(base).cast::<u32>();
            // Wait (bounded) until this slot is free: arming a slot
            // the chip still owns corrupts the transmit.
            let mut spins = 0u32;
            while d.read_volatile() & omniosk::net::OPTS_OWN != 0 {
                spins += 1;
                if spins > 10_000_000 {
                    return false;
                }
                core::hint::spin_loop();
            }
            core::ptr::copy_nonoverlapping(desc.as_ptr(), self.ring_ptr.add(base), DESC_SIZE);
            // Self-check: the bytes the chip will read must carry OWN.
            // A silent no-op here is exactly what the hardware smoke
            // could not see.
            if d.read_volatile() & omniosk::net::OPTS_OWN == 0 {
                return false;
            }
        }

        // Kick the transmit ring.
        self.w8(REG_TPPOLL, TPPOLL_NPQ);
        self.tx_slot = (slot + 1) % RING_DESCS;

        // Wait (bounded) for the chip to release the slot, so the
        // shared frame buffer is safe to overwrite on the next send.
        // SAFETY: same ring bounds as above.
        unsafe {
            let d = self.ring_ptr.add(base).cast::<u32>();
            let mut spins = 0u32;
            while d.read_volatile() & omniosk::net::OPTS_OWN != 0 {
                spins += 1;
                if spins > 10_000_000 {
                    return false;
                }
                core::hint::spin_loop();
            }
        }
        true
    }
}
