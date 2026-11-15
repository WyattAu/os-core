//! REQ-OS-007 model test: a random alloc/free op sequence applied to both
//! `PageAlloc` and a `BTreeSet` oracle must produce identical observable
//! state. This is the estate's model-based pattern (omni-ring
//! tests/model.rs) applied to the allocator.

use std::collections::BTreeSet;

use omni_heap::{HeapError, PageAlloc, PAGE_SIZE};
use proptest::prelude::*;

const BASE: usize = 0x10_000;
const PAGES: usize = 8;

/// One op: allocate, free a live page, free a garbage page, or query.
#[derive(Debug, Clone, Copy)]
enum Op {
    Alloc,
    Free(usize),
    FreeForeign,
    Query(usize),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => Just(Op::Alloc),
        3 => (0usize..PAGES).prop_map(|i| Op::Free(BASE + i * PAGE_SIZE)),
        1 => (1usize..PAGES).prop_map(|i| Op::Free(BASE + i * PAGE_SIZE - 8)), // misaligned
        1 => Just(Op::FreeForeign),
        1 => (0usize..PAGES + 2).prop_map(|i| Op::Query(BASE + i * PAGE_SIZE)),
    ]
}

fn apply_model(model: &mut BTreeSet<usize>, op: Op) -> Result<(), HeapError> {
    match op {
        Op::Alloc => {
            if model.len() == PAGES {
                Err(HeapError::Exhausted)
            } else {
                let Some(idx) = (0..PAGES).find(|i| !model.contains(&(BASE + i * PAGE_SIZE)))
                else {
                    return Err(HeapError::Exhausted);
                };
                model.insert(BASE + idx * PAGE_SIZE);
                Ok(())
            }
        }
        Op::Free(a) => {
            if a % PAGE_SIZE != 0 {
                Err(HeapError::NotAligned)
            } else if !(BASE..BASE + PAGES * PAGE_SIZE).contains(&a) {
                Err(HeapError::NotOwned)
            } else if model.remove(&a) {
                Ok(())
            } else {
                Err(HeapError::DoubleFree)
            }
        }
        Op::FreeForeign => Err(HeapError::NotOwned),
        Op::Query(_) => Ok(()),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn model_matches(ops in proptest::collection::vec(op_strategy(), 0..64)) {
        let mut backing = [0u64; 1];
        let Ok(mut heap) = PageAlloc::new(BASE, PAGES, &mut backing) else {
            // Infallible by construction (validated in unit tests).
            std::process::abort();
        };
        let mut model: BTreeSet<usize> = BTreeSet::new();

        for op in ops {
            let expected = apply_model(&mut model, op);
            let actual = match op {
                Op::Alloc => {
                    let r = heap.alloc();
                    if let Ok(a) = &r {
                        // Invariant 1: aligned and inside the managed range.
                        assert_eq!(a % PAGE_SIZE, 0, "op {op:?}: aligned");
                        assert!(
                            *a >= BASE && *a < BASE + PAGES * PAGE_SIZE,
                            "op {op:?}: in range"
                        );
                    }
                    r.map(|_| ())
                }
                Op::Free(a) => heap.free(a),
                Op::FreeForeign => heap.free(BASE + 512 * PAGE_SIZE),
                Op::Query(a) => {
                    assert_eq!(
                        heap.is_allocated(a),
                        model.contains(&a),
                        "op {op:?}: is_allocated agrees with the model"
                    );
                    Ok(())
                }
            };
            assert_eq!(actual, expected, "op {op:?}: result agrees with model");
            assert_eq!(
                heap.used_pages(),
                model.len(),
                "op {op:?}: used-page count agrees with model"
            );
        }
    }
}
