//! Cross-platform smoke for omni-heap (REQ-OS-007): a deterministic
//! alloc/free sequence with invariant checks, runnable on any target.
//! Used to validate the allocator off-host (aarch64/Android via adb):
//! `adb shell taskset -c 7 /data/local/tmp/heap-smoke`.
//! Prints `HEAP-ARM-PASS` on success; exits nonzero on any violation.

use omni_heap::{HeapError, PageAlloc, PAGE_SIZE};
use omni_ring::{slot::SlotTable, spsc::SpscRing};

const BASE: usize = 0x4000_0000;
const PAGES: usize = 64;

fn main() {
    let mut backing = [0u64; 1];
    let Ok(mut heap) = PageAlloc::new(BASE, PAGES, &mut backing) else {
        eprintln!("HEAP-ARM-FAIL: new rejected a valid layout");
        std::process::exit(1);
    };

    // First-fit: page i lands at BASE + i * PAGE_SIZE, ascending.
    let mut live: Vec<usize> = Vec::with_capacity(PAGES);
    for i in 0..PAGES {
        match heap.alloc() {
            Ok(a) if a == BASE + i * PAGE_SIZE => live.push(a),
            Ok(a) => {
                eprintln!(
                    "HEAP-ARM-FAIL: alloc {i} gave {a:#x}, wanted {:#x}",
                    BASE + i * PAGE_SIZE
                );
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("HEAP-ARM-FAIL: alloc {i} errored ({e:?})");
                std::process::exit(1);
            }
        }
    }
    assert_eq!(heap.used_pages(), PAGES, "all pages live");
    assert_eq!(
        heap.alloc(),
        Err(HeapError::Exhausted),
        "full heap is exhausted"
    );

    // Free the even-indexed pages (32 of them), then first-fit must
    // hand back the lowest freed page on every subsequent alloc.
    let mut freed: Vec<usize> = Vec::with_capacity(PAGES / 2);
    for (i, addr) in live.iter().enumerate() {
        if i % 2 == 0 {
            assert!(heap.free(*addr).is_ok(), "free of a live page");
            freed.push(*addr);
        }
    }
    assert_eq!(
        heap.used_pages(),
        PAGES - freed.len(),
        "used count tracks frees"
    );
    freed.sort_unstable();
    for want in &freed {
        match heap.alloc() {
            Ok(a) => assert_eq!(a, *want, "first-fit returns the lowest freed page"),
            Err(e) => {
                eprintln!("HEAP-ARM-FAIL: realloc errored ({e:?})");
                std::process::exit(1);
            }
        }
    }
    assert_eq!(heap.used_pages(), PAGES, "heap is whole again");

    println!("HEAP-ARM-PASS ({PAGES} pages, {PAGE_SIZE} bytes/page)");

    // Typed FIFO (REQ-OS-001 generic-payload follow-up): descriptors
    // ride the ring, payloads live in the SlotTable — io_uring's model
    // at micro scale, validated on the host target too.
    let ring: SpscRing<8> = SpscRing::new();
    let mut table: SlotTable<u64, 8> = SlotTable::new();
    for d in 0u64..4 {
        assert_eq!(table.write(d, d * 11), Some(()), "slot write");
        assert_eq!(ring.try_push(d), None, "ring absorbs 4");
    }
    for d in 0u64..4 {
        assert_eq!(ring.try_pop(), Some(d), "FIFO order");
        assert_eq!(table.take(d), Some(d * 11), "typed round-trip");
    }
    println!("FIFO-ARM-PASS");
}
