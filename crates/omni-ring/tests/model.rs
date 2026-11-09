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
