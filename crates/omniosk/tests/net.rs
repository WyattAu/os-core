//! Layout tests for the RTL8168 Tx descriptor and the UDP frame.
//!
//! This is the gate the hardware smoke cannot provide: QEMU has no
//! RTL8168 model, so a field-order inversion surfaces only as a silent
//! zero-beacon boot on real hardware (which is exactly what shipped).

use omniosk::net::{
    build_udp_frame, encode_rx_desc, encode_tx_desc, IPV4_HDR, MAX_FRAME, MIN_FRAME, OPTS_EOR,
    OPTS_FS, OPTS_LS, OPTS_OWN, UDP_HDR,
};

fn be16_at(buf: &[u8], off: usize) -> Option<u16> {
    let bytes: [u8; 2] = buf.get(off..off + 2)?.try_into().ok()?;
    Some(u16::from_be_bytes(bytes))
}

fn le32_at(buf: &[u8], off: usize) -> Option<u32> {
    let bytes: [u8; 4] = buf.get(off..off + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn le64_at(buf: &[u8], off: usize) -> Option<u64> {
    let bytes: [u8; 8] = buf.get(off..off + 8)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn ipv4_checksum_ok(hdr: &[u8]) -> bool {
    let mut sum = 0u32;
    for word in hdr.chunks_exact(2) {
        let hi = u32::from(word.first().copied().unwrap_or(0));
        let lo = u32::from(word.get(1).copied().unwrap_or(0));
        sum += (hi << 8) | lo;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    sum == 0xFFFF
}

#[test]
fn descriptor_opts1_carries_own_fs_ls_and_length() {
    let desc = encode_tx_desc(0x1234_5678, 82, false);
    let expected = OPTS_OWN | OPTS_FS | OPTS_LS | 0x0052;
    assert_eq!(le32_at(&desc, 0), Some(expected));
    assert_eq!(le32_at(&desc, 4), Some(0), "opts2 must be zero (no VLAN)");
}

#[test]
fn descriptor_addr_is_at_offset_eight() {
    let desc = encode_tx_desc(0x0000_0001_2345_6789, 60, false);
    assert_eq!(le64_at(&desc, 8), Some(0x0000_0001_2345_6789));
    // The address must not leak into opts1 (the shipped bug).
    assert_eq!(
        le32_at(&desc, 0),
        Some(OPTS_OWN | OPTS_FS | OPTS_LS | 0x003C)
    );
}

#[test]
fn descriptor_eor_marks_the_ring_end() {
    let desc = encode_tx_desc(0x1000, 60, true);
    assert_eq!(
        le32_at(&desc, 0),
        Some(OPTS_OWN | OPTS_EOR | OPTS_FS | OPTS_LS | 0x003C)
    );
}

#[test]
fn frame_layout_matches_the_headers() {
    let mut buf = [0u8; MAX_FRAME];
    let payload = b"OMNIOSK BOOT enter";
    let total = build_udp_frame(
        &mut buf,
        [0x6C, 0x4B, 0x90, 0x48, 0xC3, 0x65],
        [192, 168, 1, 191],
        [192, 168, 1, 1],
        40_969,
        9_933,
        payload,
    )
    .unwrap_or(0);
    assert_eq!(total, MIN_FRAME + payload.len());
    assert_eq!(buf.get(0..6), Some([0xFF; 6].as_slice()));
    assert_eq!(
        buf.get(6..12),
        Some([0x6C, 0x4B, 0x90, 0x48, 0xC3, 0x65].as_slice())
    );
    assert_eq!(buf.get(12..14), Some([0x08, 0x00].as_slice()));
    assert_eq!(buf.get(14..15), Some([0x45].as_slice()));
    assert_eq!(
        be16_at(&buf, 16),
        Some(u16::try_from(IPV4_HDR + UDP_HDR + payload.len()).unwrap_or(0))
    );
    assert_eq!(buf.get(23..24), Some([17].as_slice()));
    assert_eq!(buf.get(26..30), Some([192, 168, 1, 191].as_slice()));
    assert_eq!(buf.get(30..34), Some([192, 168, 1, 1].as_slice()));
    // The UDP header follows the IPv4 header directly: 14 + 20 = 34.
    assert_eq!(be16_at(&buf, 34), Some(40_969));
    assert_eq!(be16_at(&buf, 36), Some(9_933));
    assert_eq!(
        be16_at(&buf, 38),
        Some(u16::try_from(UDP_HDR + payload.len()).unwrap_or(0))
    );
    assert_eq!(buf.get(40..42), Some([0, 0].as_slice()));
    assert_eq!(buf.get(42..42 + payload.len()), Some(payload.as_slice()));
    assert!(ipv4_checksum_ok(buf.get(14..34).unwrap_or(&[])));
}

#[test]
fn oversized_payload_is_rejected() {
    let mut buf = [0u8; MAX_FRAME + 32];
    let payload = [0u8; MAX_FRAME];
    assert_eq!(
        build_udp_frame(&mut buf, [0; 6], [0; 4], [0; 4], 1, 1, &payload),
        None
    );
}

#[test]
fn short_buffer_is_rejected() {
    let mut buf = [0u8; MIN_FRAME - 1];
    assert_eq!(
        build_udp_frame(&mut buf, [0; 6], [0; 4], [0; 4], 1, 1, &[]),
        None
    );
}

// ---- RX descriptor contract ----

fn rx_opts1(desc: &[u8; 16]) -> Option<u32> {
    let b: [u8; 4] = desc.get(0..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(b))
}

fn rx_addr(desc: &[u8; 16]) -> Option<u64> {
    let b: [u8; 8] = desc.get(8..16)?.try_into().ok()?;
    Some(u64::from_le_bytes(b))
}

#[test]
fn rx_desc_init_carries_own_buffer_size_and_ring_end() {
    let desc = encode_rx_desc(0x0001_2000, 4096, true);
    assert_eq!(
        rx_opts1(&desc),
        Some(omniosk::net::RX_OWN | omniosk::net::RX_RING_END | 4096)
    );
    assert_eq!(rx_addr(&desc), Some(0x0001_2000));
}

#[test]
fn rx_desc_done_clears_own_and_reports_len_frags() {
    // The chip deposits a frame and clears OWN, writing the length
    // (incl. FCS) plus First|Last fragment flags.
    let mut desc = encode_rx_desc(0x1000, 4096, false);
    let b: &mut [u8] = desc.as_mut_slice();
    let done = omniosk::net::RX_FIRST_FRAG | omniosk::net::RX_LAST_FRAG | 0x59;
    let (head, _rest) = b.split_at_mut(4);
    head.copy_from_slice(&done.to_le_bytes());
    assert_eq!(omniosk::net::decode_rx_done(done), Some((0x59, true, true)));
}

#[test]
fn rx_desc_owned_by_chip_reads_as_pending() {
    let desc = encode_rx_desc(0x1000, 4096, false);
    assert_eq!(rx_opts1(&desc).and_then(omniosk::net::decode_rx_done), None);
}
