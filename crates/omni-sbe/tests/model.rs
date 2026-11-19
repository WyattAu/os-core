//! REQ-OS (pillar 208) model tests for the SBE decoder: untrusted
//! frames must never panic, every successful read stays in bounds, and
//! group/varData extents declared past the frame are rejected.

use omni_sbe::SbeDecoder;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn random_frames_never_panic(data in proptest::collection::vec(any::<u8>(), 0..64)) {
        let d = SbeDecoder::new(&data);
        let _ = d.message_header();
        for off in 0..data.len() + 2 {
            let _ = d.scalar::<8>(off);
            let _ = d.u16_at(off);
            let _ = d.group_at(off);
            let _ = d.var_data(off);
        }
    }

    /// Header decode is exact: the four u16 fields come back in wire
    /// order from an 8-byte frame.
    #[test]
    fn header_fields_round_trip(
        bl in any::<u16>(), tid in any::<u16>(), sid in any::<u16>(), ver in any::<u16>()
    ) {
        let f = [bl.to_le_bytes(), tid.to_le_bytes(), sid.to_le_bytes(), ver.to_le_bytes()].concat();
        let d = SbeDecoder::new(&f);
        let Some(h) = d.message_header() else {
            std::process::abort();
        };
        prop_assert_eq!(h.block_length, bl);
        prop_assert_eq!(h.template_id, tid);
        prop_assert_eq!(h.schema_id, sid);
        prop_assert_eq!(h.version, ver);
    }

    /// A repeating group whose declared extent exceeds the frame is
    /// rejected — the hostile-declared-length case.
    #[test]
    fn group_overflow_rejected(num_in_group in 4u16..=64u16, body in any::<Vec<u8>>()) {
        let mut f = Vec::new();
        f.extend_from_slice(&6u16.to_le_bytes()); // entry block length
        f.extend_from_slice(&num_in_group.to_le_bytes());
        f.extend_from_slice(&body);
        let d = SbeDecoder::new(&f);
        // The frame = 12 + body.len() bytes; the entries span bytes
        // 16..(16 + 6*count). They fit iff 16 + 6*count <= 12 + body.
        // The frame = 8-byte header + 4-byte dims at 8 + body; the
        // entries span bytes 12..(12 + 6*count). They fit iff
        // 12 + 6*count <= 12 + body, i.e. body >= 6*count.
        let expected_fit = body.len() >= 6 * num_in_group as usize;
        if d.group_at(8).is_some() != expected_fit {
            println!(
                "MISMATCH: body.len={} num_in_group={} decoder_accepted={} expected_fit={}",
                body.len(),
                num_in_group,
                d.group_at(8).is_some(),
                expected_fit
            );
        }
        prop_assert_eq!(d.group_at(0).is_some(), expected_fit,
            "group acceptance must match the declared extent");
    }

    /// varData length prefixes running past the frame are rejected.
    #[test]
    fn var_data_overflow_rejected(len in 200u8..=255u8, body in any::<Vec<u8>>()) {
        let mut f = Vec::new();
        f.extend_from_slice(&[0u8; 8]); // header
        f.extend_from_slice(&[len]);    // declares 200-255 bytes
        f.extend_from_slice(&body);     // body is shorter than that
        let d = SbeDecoder::new(&f);
        prop_assert!(d.var_data(8).is_none(),
            "a varData length past the frame must be rejected");
    }
}
