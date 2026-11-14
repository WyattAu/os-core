//! REQ-OS-006 model tests for the block seam: random LBA + random
//! 512-byte payloads against [`BlockRam`], checking the write/read-back
//! contract, range rejection, sector independence, and a multi-sector
//! sweep. Same model-based pattern as omni-heap and omni-ring.

use omni_block::{BlockError, BlockRam, LBA_SIZE};
use proptest::prelude::*;

const SECTORS: usize = 8;
const LAST_LBA: u64 = (SECTORS - 1) as u64;

/// A uniformly random 512-byte sector, built without indexing or
/// panicking (the workspace denies both, tests included).
fn payload() -> impl Strategy<Value = [u8; LBA_SIZE]> {
    proptest::collection::vec(any::<u8>(), LBA_SIZE).prop_map(|bytes| {
        let mut sector = [0; LBA_SIZE];
        sector
            .iter_mut()
            .zip(bytes)
            .for_each(|(out, byte)| *out = byte);
        sector
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Invariant 2: a write is durable — read back matches, byte for byte.
    #[test]
    fn write_then_read_back_matches(lba in 0u64..=LAST_LBA, payload in payload()) {
        let mut disk = BlockRam::<SECTORS>::new();
        assert_eq!(disk.write_lba(lba, &payload), Ok(()));
        let mut readback = [0u8; LBA_SIZE];
        assert_eq!(disk.read_lba(lba, &mut readback), Ok(()));
        assert_eq!(readback, payload, "lba {lba}: read-back must match write");
        // The smoke contract expresses the same property as one call.
        assert_eq!(disk.verify_roundtrip(lba, &payload), Ok(()));
    }

    /// Invariant 1: out-of-range LBAs are `OutOfRange`, never a panic,
    /// for both directions; the device is untouched.
    #[test]
    fn out_of_range_lbas_error(lba in (LAST_LBA + 1)..=(LAST_LBA + 64), payload in payload()) {
        let mut disk = BlockRam::<SECTORS>::new();
        assert_eq!(disk.read_lba(lba, &mut [0; LBA_SIZE]), Err(BlockError::OutOfRange));
        assert_eq!(disk.write_lba(lba, &payload), Err(BlockError::OutOfRange));
        // Unrepresentable-on-usize LBAs take the same honest path.
        assert_eq!(
            disk.write_lba(u64::MAX, &payload),
            Err(BlockError::OutOfRange)
        );
    }

    /// Invariant 3: writes to one LBA never disturb another.
    #[test]
    fn cross_lba_independence(
        a in 0u64..=LAST_LBA,
        b in 0u64..=LAST_LBA,
        payload_a in payload(),
        payload_b in payload(),
    ) {
        let mut disk = BlockRam::<SECTORS>::new();
        assert_eq!(disk.write_lba(a, &payload_a), Ok(()));
        assert_eq!(disk.write_lba(b, &payload_b), Ok(()));

        let mut buf_a = [0u8; LBA_SIZE];
        let mut buf_b = [0u8; LBA_SIZE];
        assert_eq!(disk.read_lba(a, &mut buf_a), Ok(()));
        assert_eq!(disk.read_lba(b, &mut buf_b), Ok(()));
        let expected_a = if a == b { &payload_b } else { &payload_a };
        assert_eq!(buf_a, *expected_a, "last write to lba {a} wins");
        assert_eq!(buf_b, payload_b, "lba {b} holds its own payload");
    }

    /// Multi-sector sweep: fill every sector with a distinct pattern,
    /// then verify all of them survive. The marker byte is a wrapping
    /// `u8` counter (`u8` ranges wrap by design), so no casts are needed.
    #[test]
    fn multi_sector_write_loop(base_byte in any::<u8>()) {
        let mut disk = BlockRam::<SECTORS>::new();
        let mut payloads = [[0u8; LBA_SIZE]; SECTORS];
        let mut marker = base_byte;
        for slot in &mut payloads {
            let mut byte = marker;
            for out in slot.iter_mut() {
                *out = byte;
                byte = byte.wrapping_add(1);
            }
            marker = marker.wrapping_add(1);
        }
        for (lba, payload) in payloads.iter().enumerate() {
            assert_eq!(
                disk.verify_roundtrip(lba as u64, payload),
                Ok(()),
                "sector {lba}: roundtrip"
            );
        }
        // Full sweep leaves every sector holding its own pattern.
        for (lba, expected) in payloads.iter().enumerate() {
            let mut buf = [0u8; LBA_SIZE];
            assert_eq!(disk.read_lba(lba as u64, &mut buf), Ok(()));
            assert_eq!(&buf, expected, "sector {lba}: holds its own pattern");
        }
    }

    /// Invariant 4: a fresh device reads as all zeros in every sector.
    #[test]
    fn fresh_device_is_zeroed(lba in 0u64..=LAST_LBA) {
        let disk = BlockRam::<SECTORS>::new();
        let mut buf = [0xFF; LBA_SIZE];
        assert_eq!(disk.read_lba(lba, &mut buf), Ok(()));
        assert!(buf.iter().all(|&b| b == 0), "lba {lba}: fresh = zeros");
    }
}
