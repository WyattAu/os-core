//! Property model: both rings behave as FIFO queues (REQ-OS-001).

use omni_ring::{mpmc::MpmcRing, spsc::SpscRing};
use proptest::prelude::*;

proptest! {
    #[test]
    fn spsc_matches_fifo_model(vals in proptest::collection::vec(any::<u64>(), 0..=8_usize)) {
        let ring: SpscRing<8> = SpscRing::new();
        for &v in &vals {
            assert_eq!(ring.try_push(v), None, "cap 8 absorbs at most 8 pushes");
        }
        prop_assert_eq!(ring.len(), vals.len());
        for v in &vals {
            prop_assert_eq!(ring.try_pop(), Some(*v));
        }
        prop_assert!(ring.is_empty());
    }

    #[test]
    fn mpmc_matches_fifo_model(vals in proptest::collection::vec(any::<u64>(), 0..=8_usize)) {
        let ring: MpmcRing<u64> = MpmcRing::new(8);
        for &v in &vals {
            assert!(ring.push(v).is_ok());
        }
        prop_assert_eq!(ring.len(), vals.len());
        for v in &vals {
            prop_assert_eq!(ring.pop(), Some(*v));
        }
    }
}

/// Typed FIFO end-to-end (REQ-OS-001 generic-payload follow-up): values
/// written into a `SlotTable` at descriptor `d`, transported by the ring,
/// and read back after popping — come out in push order with the exact
/// typed payload. This is io_uring's model at micro scale.
#[test]
fn spsc_slot_table_typed_fifo() {
    use omni_ring::{slot::SlotTable, spsc::SpscRing};

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Task {
        id: u16,
        flags: u8,
    }

    let ring: SpscRing<8> = SpscRing::new();
    let mut table: SlotTable<Task, 8> = SlotTable::new();

    for d in 0u64..4 {
        let task = Task {
            id: (d as u16) * 7,
            flags: 0xA0 | d as u8,
        };
        assert_eq!(table.write(d, task), Some(()));
        assert_eq!(ring.try_push(d), None, "ring absorbs 4 descriptors");
    }

    for d in 0u64..4 {
        let got = ring.try_pop();
        assert_eq!(got, Some(d), "FIFO order");
        let task = table.take(d);
        assert_eq!(
            task,
            Some(Task {
                id: (d as u16) * 7,
                flags: 0xA0 | d as u8,
            }),
            "typed payload round-trips"
        );
    }
}
