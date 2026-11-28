//! `omni-net` wire surface — the RTL8168 transmit descriptor and the
//! Ethernet/IPv4/UDP frame, `no_std` and panic-free.
//!
//! The transmit driver in `kernels/omniosk-kernel/src/rtl8168.rs` used
//! to hand-encode two byte-level contracts: the 16-byte Tx descriptor
//! and the frame. Both were written against the Linux `r8169`
//! definitions but never gated — QEMU ships no RTL8168 model, so the
//! only test was the unattended hardware smoke, and a field-order
//! inversion (frame address in `opts1`, flags in `opts2`) sent every
//! beacon nowhere while looking like success.
//!
//! This module owns the encoding so the host tests pin the layout and
//! the kernel only copies bytes. The descriptor layout is the Linux
//! `r8169` `struct TxDesc`:
//!
//! ```text
//! 0x00  opts1  u32 LE   OWN(31) | EOR(30) | FS(29) | LS(28) | length(15:0)
//! 0x04  opts2  u32 LE   VLAN tag (0 here)
//! 0x08  addr   u64 LE   frame physical address
//! ```
//!
//! The frame is Ethernet II (14) | IPv4 (20) | UDP (8) | payload, the
//! IPv4 header checksum filled, the UDP checksum left zero (legal for
//! IPv4: "checksum not computed").

/// Ethernet II header size in bytes.
pub const ETH_HDR: usize = 14;
/// IPv4 header size in bytes (no options).
pub const IPV4_HDR: usize = 20;
/// UDP header size in bytes.
pub const UDP_HDR: usize = 8;
/// Smallest frame this builder produces (no payload).
pub const MIN_FRAME: usize = ETH_HDR + IPV4_HDR + UDP_HDR;
/// Largest Ethernet frame, excluding the FCS.
pub const MAX_FRAME: usize = 1500;

/// Tx descriptor `opts1`: the chip owns the descriptor.
pub const OPTS_OWN: u32 = 0x8000_0000;
/// Tx descriptor `opts1`: end of the descriptor ring.
pub const OPTS_EOR: u32 = 0x4000_0000;
/// Tx descriptor `opts1`: first segment of a frame.
pub const OPTS_FS: u32 = 0x2000_0000;
/// Tx descriptor `opts1`: last segment of a frame.
pub const OPTS_LS: u32 = 0x1000_0000;

/// Encodes one RTL8168 transmit descriptor.
///
/// `frame_phys` is the DMA address the chip reads, `len` is the frame
/// length (the 16-bit length field), and `eor` marks the last
/// descriptor of the ring so the chip wraps to slot 0.
#[must_use]
pub fn encode_tx_desc(frame_phys: u64, len: u16, eor: bool) -> [u8; 16] {
    let mut opts1 = OPTS_OWN | OPTS_FS | OPTS_LS | u32::from(len);
    if eor {
        opts1 |= OPTS_EOR;
    }
    let mut desc = [0u8; 16];
    let (head, rest) = desc.split_at_mut(4);
    head.copy_from_slice(&opts1.to_le_bytes());
    // `opts2` (VLAN) stays zero: no tag on these frames.
    let (_opts2, addr) = rest.split_at_mut(4);
    addr.copy_from_slice(&frame_phys.to_le_bytes());
    desc
}

/// Writes `bytes` into `buf` at `off` when the range fits; an
/// out-of-range write is ignored (the caller fixes `total` first).
fn put(buf: &mut [u8], off: usize, bytes: &[u8]) {
    if let Some(dst) = buf.get_mut(off..off.saturating_add(bytes.len())) {
        dst.copy_from_slice(bytes);
    }
}

/// IPv4 header checksum (RFC 1071) over a 20-byte header.
fn ipv4_checksum(hdr: &[u8]) -> u16 {
    let mut sum = 0u32;
    for word in hdr.chunks_exact(2) {
        let hi = u32::from(word.first().copied().unwrap_or(0));
        let lo = u32::from(word.get(1).copied().unwrap_or(0));
        sum += (hi << 8) | lo;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    // After folding, `sum` fits a u16 by construction.
    !u16::try_from(sum).unwrap_or(0)
}

/// Builds a complete Ethernet/IPv4/UDP frame into `buf`.
///
/// Returns the frame length on success, `None` when the payload does
/// not fit in [`MAX_FRAME`] or `buf` is shorter than the frame.
///
/// The destination MAC is broadcast (the kernel has no ARP stack), the
/// source MAC is the NIC's hardware address, and the UDP checksum is
/// zero (not computed; legal for IPv4).
#[must_use]
pub fn build_udp_frame(
    buf: &mut [u8],
    mac: [u8; 6],
    from_ip: [u8; 4],
    to_ip: [u8; 4],
    from_port: u16,
    to_port: u16,
    payload: &[u8],
) -> Option<usize> {
    let total = MIN_FRAME.checked_add(payload.len())?;
    if total > MAX_FRAME {
        return None;
    }
    let frame = buf.get_mut(..total)?;
    let (eth, l3) = frame.split_at_mut(ETH_HDR);
    let (ip, l4) = l3.split_at_mut(IPV4_HDR);
    let (udp, body) = l4.split_at_mut(UDP_HDR);

    // ---- Ethernet II: broadcast destination, our source, IPv4 ----
    put(eth, 0, &[0xFF; 6]);
    put(eth, 6, &mac);
    put(eth, 12, &[0x08, 0x00]);

    // ---- IPv4: v4/IHL 5, DF, TTL 64, protocol UDP ----
    let ip_total = u16::try_from(IPV4_HDR + UDP_HDR + payload.len()).ok()?;
    put(ip, 0, &[0x45, 0x00]);
    put(ip, 2, &ip_total.to_be_bytes());
    put(ip, 4, &[0x00, 0x01]); // identification
    put(ip, 6, &[0x40, 0x00]); // don't fragment
    put(ip, 8, &[64]); // TTL
    put(ip, 9, &[17]); // protocol: UDP
    put(ip, 10, &[0x00, 0x00]); // checksum field zeroed for the sum
    put(ip, 12, &from_ip);
    put(ip, 16, &to_ip);
    let checksum = ipv4_checksum(ip);
    put(ip, 10, &checksum.to_be_bytes());

    // ---- UDP: ports, length, checksum 0 (not computed) ----
    let udp_len = u16::try_from(UDP_HDR + payload.len()).ok()?;
    put(udp, 0, &from_port.to_be_bytes());
    put(udp, 2, &to_port.to_be_bytes());
    put(udp, 4, &udp_len.to_be_bytes());
    put(udp, 6, &[0x00, 0x00]);
    put(body, 0, payload);

    Some(total)
}

// ---- RX descriptor contract (RTL8168G, DEFAULT descriptor type;
// Linux r8169_main.c: pkt_size = status & GENMASK(13, 0), OWN = bit 31,
// RingEnd = bit 30, First/LastFrag = bits 29/28. Init hands
// OWN | buffer-size to the chip; the chip clears OWN on deposit.) ----

/// Rx descriptor `opts1`: the chip owns the buffer.
pub const RX_OWN: u32 = 0x8000_0000;
/// Rx descriptor `opts1`: last descriptor of the ring.
pub const RX_RING_END: u32 = 0x4000_0000;
/// Rx descriptor `opts1`: this buffer holds the first fragment.
pub const RX_FIRST_FRAG: u32 = 0x2000_0000;
/// Rx descriptor `opts1`: this buffer holds the last fragment.
pub const RX_LAST_FRAG: u32 = 0x1000_0000;
/// Rx descriptor `opts1` bits 13:0: the deposited frame length
/// including the 4-byte FCS.
pub const RX_LEN_MASK: u32 = 0x0000_3FFF;

/// Encodes an RX descriptor handed to the chip (init state): OWN set,
/// the length field carrying the buffer capacity, optional ring-end.
#[must_use]
pub fn encode_rx_desc(buf_phys: u64, buf_size: u32, eor: bool) -> [u8; 16] {
    let mut opts1 = RX_OWN | (buf_size & RX_LEN_MASK);
    if eor {
        opts1 |= RX_RING_END;
    }
    let mut desc = [0u8; 16];
    let (head, rest) = desc.split_at_mut(4);
    head.copy_from_slice(&opts1.to_le_bytes());
    let (_opts2, addr) = rest.split_at_mut(4);
    addr.copy_from_slice(&buf_phys.to_le_bytes());
    desc
}

/// Decodes a completed RX descriptor: `(frame length incl. FCS, first
/// fragment, last fragment)`. `None` while the chip still owns the
/// descriptor (no frame deposited yet).
#[must_use]
pub fn decode_rx_done(opts1: u32) -> Option<(usize, bool, bool)> {
    if opts1 & RX_OWN != 0 {
        return None;
    }
    Some((
        (opts1 & RX_LEN_MASK) as usize,
        opts1 & RX_FIRST_FRAG != 0,
        opts1 & RX_LAST_FRAG != 0,
    ))
}
