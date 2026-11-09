//! Loom model check: MPMC ring loses nothing under producer/consumer
//! interleavings (REQ-OS-001).

#![cfg(feature = "loom")]

use omni_ring::mpmc::MpmcRing;
use std::sync::Arc;

#[test]
fn two_producers_one_consumer_no_loss() {
    loom::model(|| {
        let ring = Arc::new(MpmcRing::new(2));

        let producers: Vec<_> = [10_u64, 20]
            .iter()
            .map(|&v| {
                let r = Arc::clone(&ring);
                loom::thread::spawn(move || {
                    // CAP = 2 with two single pushes: never full.
                    assert!(r.push(v).is_ok());
                })
            })
            .collect();

        let consumer_ring = Arc::clone(&ring);
        let consumer = loom::thread::spawn(move || {
            let mut got: Vec<u64> = Vec::new();
            while got.len() < 2 {
                if let Some(v) = consumer_ring.pop() {
                    got.push(v);
                } else {
                    loom::hint::spin_loop();
                }
            }
            got.sort_unstable();
            got
        });

        for h in producers {
            assert_eq!(h.join().ok(), Some(()));
        }
        assert_eq!(consumer.join().ok(), Some(vec![10, 20]));
    });
}
