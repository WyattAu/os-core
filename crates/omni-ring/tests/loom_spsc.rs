//! Loom model check: SPSC FIFO order holds under every thread interleaving,
//! including slot wraparound (REQ-OS-001).

#![cfg(feature = "loom")]

use omni_ring::spsc::SpscRing;
use std::sync::Arc;

#[test]
fn fifo_across_all_interleavings_with_wraparound() {
    loom::model(|| {
        let ring = Arc::new(SpscRing::<2>::new());
        let producer_ring = Arc::clone(&ring);
        let consumer_ring = Arc::clone(&ring);

        let producer = loom::thread::spawn(move || {
            for v in [1_u64, 2, 3] {
                // Spins only while the ring is full; the consumer's pops
                // always free a slot, so this is bounded under loom.
                while producer_ring.try_push(v).is_some() {
                    loom::hint::spin_loop();
                }
            }
        });

        let consumer = loom::thread::spawn(move || {
            let mut got: Vec<u64> = Vec::new();
            while got.len() < 3 {
                if let Some(v) = consumer_ring.try_pop() {
                    got.push(v);
                } else {
                    loom::hint::spin_loop();
                }
            }
            got
        });

        assert_eq!(producer.join().ok(), Some(()));
        assert_eq!(consumer.join().ok(), Some(vec![1, 2, 3]));
    });
}
