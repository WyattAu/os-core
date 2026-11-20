//! omni-md pipeline tests: the canned SBE frame decodes to the exact
//! tick, hostile frames are rejected, and the fan-out delivers in
//! publish order (REQ-OS-010 data plane).

use omni_md::{fanout::Fanout, frame_to_tick, FRAME_LEN, TICK_SCHEMA_ID, TICK_TEMPLATE_ID};
use omni_sbe::SbeEncoder;
use omni_tick::Tick;

/// Builds a valid trade-tick frame for `tick` via the real SBE
/// encoder (the same wire omni-io will carry).
fn encode_frame(t: &Tick) -> [u8; FRAME_LEN] {
    let mut frame = [0u8; FRAME_LEN];
    let mut enc = SbeEncoder::new(&mut frame);
    enc.write_header(32, TICK_TEMPLATE_ID, TICK_SCHEMA_ID, 0);
    let body_ok = enc.put_u32(8, t.instrument)
        && enc.put_u32(12, t.flags)
        && enc.put_u64(16, t.bid_px)
        && enc.put_u64(24, t.ask_px)
        && enc.put_u64(32, t.ts_ns);
    assert!(body_ok, "body scalars fit the frame by construction");
    frame
}

#[test]
fn canned_frame_decodes_to_exact_tick() {
    let t = Tick {
        ts_ns: 1_700_000_000_000_000_000,
        instrument: 42,
        flags: 1,
        bid_px: 4_295_032_500,
        ask_px: 4_295_033_000,
    };
    let frame = encode_frame(&t);
    let back = frame_to_tick(&frame);
    assert!(back.is_some(), "valid frame decodes");
    assert_eq!(back.unwrap_or(t), t, "round trip exact");
}

#[test]
fn hostile_frames_rejected() {
    let t = Tick {
        ts_ns: 1,
        instrument: 1,
        flags: 0,
        bid_px: 1,
        ask_px: 2,
    };
    let frame = encode_frame(&t);

    // Short frame: the body would run past the end.
    assert!(frame_to_tick(&frame[..FRAME_LEN - 1]).is_none());
    // Foreign template: not our schema's contract.
    let mut foreign = frame;
    let mut enc_f = SbeEncoder::new(&mut foreign);
    assert!(enc_f.put_u16(2, 9999), "template id field at offset 2");
    assert!(
        frame_to_tick(&foreign).is_none(),
        "foreign template rejected"
    );
    // Wrong block length: header lies about the body extent.
    let mut lying = frame;
    let mut enc_b = SbeEncoder::new(&mut lying);
    assert!(enc_b.put_u16(0, 99), "block length field at offset 0");
    assert!(frame_to_tick(&lying).is_none(), "lying header rejected");
}

#[test]
fn fanout_delivers_in_publish_order() {
    let mut fan: Fanout<8> = Fanout::new();
    let ticks: [Tick; 5] = core::array::from_fn(|i| Tick {
        ts_ns: 100 + i as u64,
        instrument: 42,
        flags: u32::try_from(i).unwrap_or_default(),
        bid_px: 1000 + i as u64,
        ask_px: 1010 + i as u64,
    });
    for (i, t) in ticks.iter().enumerate() {
        assert!(fan.publish(i as u64, t), "publish {i} of 5 fits");
    }
    assert!(!fan.publish(99, &ticks[0]), "full fan-out backpressures");
    assert_eq!(fan.len(), 5);
    for (i, want) in ticks.iter().enumerate() {
        let got = fan.drain_one();
        assert_eq!(got.as_ref(), Some(want), "drain order {i}");
    }
    assert!(fan.is_empty(), "drained to empty");
    assert!(fan.drain_one().is_none(), "empty drain is None");
}
