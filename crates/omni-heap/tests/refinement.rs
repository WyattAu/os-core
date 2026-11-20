//! Phase-3 refinement oracle (ADR-0014 pattern): executes
//! `heap_page_aligned` + `heap_page_in_range` (proofs/Proofs.lean,
//! REQ-OS-007) against `PageAlloc`. Every handed-out page must be
//! 4096-aligned for an aligned base and sit inside the managed range;
//! exactly `pages` allocations succeed, then exhaustion.

use omni_heap::{PageAlloc, PAGE_SIZE};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Executes both theorems over a random *aligned* base: every
    /// `alloc()` address satisfies `(base + idx * 4096) % 4096 == 0`
    /// (`heap_page_aligned`) with `idx < pages`
    /// (`heap_page_in_range`), and the `pages`-th allocation is
    /// exhausted — the allocator never hands out a page past the
    /// managed range.
    #[test]
    fn alloc_refines_page_layout(
        base_page in any::<u32>(),
        pages in 1usize..64usize,
    ) {
        let base = base_page as usize * PAGE_SIZE;
        let mut backing = vec![0u64; pages.div_ceil(64)];
        let Ok(mut heap) = PageAlloc::new(base, pages, &mut backing) else {
            // Infallible by construction (validated in unit tests).
            std::process::abort();
        };

        for i in 0..pages {
            let Ok(addr) = heap.alloc() else {
                // Early exhaustion is a property failure; model.rs
                // precedent for failing outside prop_assert reach.
                std::process::abort();
            };
            prop_assert_eq!(addr % PAGE_SIZE, 0, "heap_page_aligned for idx {}", i);
            prop_assert!(addr >= base, "page at or above base");
            prop_assert!(
                addr < base + pages * PAGE_SIZE,
                "heap_page_in_range for idx {}",
                i
            );
        }
        let exhausted = heap.alloc();
        prop_assert!(exhausted.is_err(), "exactly `pages` allocations succeed");
    }
}
