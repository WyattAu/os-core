//! Fuzz: SPSC ring must never lose, duplicate, or reorder descriptors.

#![no_main]

use libfuzzer_sys::fuzz_target;
use omni_ring::spsc::SpscRing;
use std::collections::VecDeque;

fuzz_target!(|data: &[u8]| {
    let ring: SpscRing<8> = SpscRing::new();
    let mut model: VecDeque<u64> = VecDeque::new();

    for byte in data.iter() {
        match byte & 1 {
            0 => {
                let v = (*byte as u64) << 8 | 0xAB;
                if model.len() < 8 {
                    assert_eq!(ring.try_push(v), None, "not full: push must succeed");
                    model.push_back(v);
                }
            }
            _ => match model.pop_front() {
                Some(expected) => assert_eq!(ring.try_pop(), Some(expected), "FIFO"),
                None => assert_eq!(ring.try_pop(), None, "empty: pop must fail"),
            },
        }
    }
    // Drained ring must match the model exactly.
    while let Some(expected) = model.pop_front() {
        assert_eq!(ring.try_pop(), Some(expected), "drain FIFO");
    }
    assert_eq!(ring.try_pop(), None, "drained ring must be empty");
});
