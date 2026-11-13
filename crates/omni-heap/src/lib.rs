//! # omni-heap — deterministic fixed-page allocator (REQ-OS-007)
//!
//! A bitmap-backed page allocator for the omniosk microcore: `no_std`,
//! allocation-free, panic-free (every failure mode is a `Result`), and
//! deterministic — allocation is first-fit over an ascending scan, so a
//! given op sequence yields identical addresses on every boot. That
//! determinism is what makes the property tests and the Lean layout
//! theorems (`proofs/Proofs.lean`) meaningful for kernel use.
//!
//! The bitmap is **borrowed, not owned**: the caller supplies the
//! backing words (a `static` in the kernel, a stack array in tests), so
//! the allocator never allocates and can run before any heap exists.
//!
//! Invariants (each is asserted by the model tests and, where cheap,
//! proved in Lean):
//!
//! 1. `alloc()` returns `base + idx * PAGE_SIZE` with `idx < pages` —
//!    inside the managed range and 4 KiB-aligned for an aligned base.
//! 2. An address is freeable exactly once; a second free is
//!    [`HeapError::DoubleFree`], not silent corruption.
//! 3. Addresses outside the managed range are [`HeapError::NotOwned`].
//! 4. Exhaustion is [`HeapError::Exhausted`], never a panic.

#![no_std]
#![deny(unsafe_code)]

/// Page size managed by this allocator (matches the `NVMe` driver's DMA
/// pages and x86-64's small-page granularity).
pub const PAGE_SIZE: usize = 4096;

/// Every fallible operation's failure mode. No panics: the estate
/// posture denies the panic family, and a microcore cannot afford
/// unwinding on the allocation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapError {
    /// `free` of a non-page-aligned address.
    NotAligned,
    /// `free` of an address outside the managed range.
    NotOwned,
    /// `free` of a page that is not currently allocated.
    DoubleFree,
    /// `alloc` with no free page left.
    Exhausted,
    /// Constructor called with a misaligned base, zero pages, or a
    /// bitmap too small for the page count.
    BadLayout,
}

/// Bitmap-backed page allocator over the fixed range
/// `[base, base + pages * PAGE_SIZE)`.
///
/// Borrow-based: the bitmap words come from the caller (`static mut`
/// backing array in the kernel, a stack array in tests). No methods
/// panic; no method allocates.
#[derive(Debug)]
pub struct PageAlloc<'a> {
    base: usize,
    pages: usize,
    bitmap: &'a mut [u64],
    used: usize,
}

impl<'a> PageAlloc<'a> {
    /// Manages `[base, base + pages * PAGE_SIZE)`, using `bitmap` as the
    /// allocation map. The base must be page-aligned and the bitmap must
    /// have at least `ceil(pages / 64)` words.
    ///
    /// # Errors
    ///
    /// [`HeapError::BadLayout`] if the base is misaligned, `pages` is
    /// zero, or the bitmap is too small for the page count.
    pub fn new(base: usize, pages: usize, bitmap: &'a mut [u64]) -> Result<Self, HeapError> {
        if base % PAGE_SIZE != 0 || pages == 0 || bitmap.len() < pages.div_ceil(64) {
            return Err(HeapError::BadLayout);
        }
        Ok(Self {
            base,
            pages,
            bitmap,
            used: 0,
        })
    }

    /// Allocates one page: first-fit over an ascending scan, returning
    /// `base + idx * PAGE_SIZE`.
    ///
    /// # Errors
    ///
    /// [`HeapError::Exhausted`] when no free page remains.
    pub fn alloc(&mut self) -> Result<usize, HeapError> {
        if self.used >= self.pages {
            return Err(HeapError::Exhausted);
        }
        for (word, bits) in self.bitmap.iter_mut().enumerate() {
            let free = !*bits;
            if free == 0 {
                continue;
            }
            let bit = free.trailing_zeros() as usize;
            let idx = word * 64 + bit;
            if idx >= self.pages {
                // Only reachable in the trailing word of a full heap;
                // the used-count guard above already handled that case.
                return Err(HeapError::Exhausted);
            }
            *bits |= 1 << bit;
            self.used += 1;
            return Ok(self.base + idx * PAGE_SIZE);
        }
        Err(HeapError::Exhausted)
    }

    /// Frees a previously allocated page. Misaligned, out-of-range and
    /// double frees are all rejected.
    ///
    /// # Errors
    ///
    /// [`HeapError::NotAligned`] for a non-page-aligned address,
    /// [`HeapError::NotOwned`] for an address outside the managed range,
    /// [`HeapError::DoubleFree`] for a page that is not live.
    pub fn free(&mut self, addr: usize) -> Result<(), HeapError> {
        if addr % PAGE_SIZE != 0 {
            return Err(HeapError::NotAligned);
        }
        let last = self.base + self.pages * PAGE_SIZE;
        if addr < self.base || addr >= last {
            return Err(HeapError::NotOwned);
        }
        let idx = (addr - self.base) / PAGE_SIZE;
        let (word, bit) = (idx / 64, idx % 64);
        // Unreachable by construction: `new` sized the bitmap for
        // `pages`, and `idx < pages`. NotOwned is the nearest honest
        // error if that invariant were ever broken.
        let bits = self.bitmap.get_mut(word).ok_or(HeapError::NotOwned)?;
        if *bits & (1 << bit) == 0 {
            return Err(HeapError::DoubleFree);
        }
        *bits &= !(1 << bit);
        self.used -= 1;
        Ok(())
    }

    /// Pages currently allocated.
    #[must_use]
    pub fn used_pages(&self) -> usize {
        self.used
    }

    /// Pages managed in total.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.pages
    }

    /// Whether `addr` is a live allocation of this heap.
    #[must_use]
    pub fn is_allocated(&self, addr: usize) -> bool {
        if addr % PAGE_SIZE != 0 {
            return false;
        }
        let last = self.base + self.pages * PAGE_SIZE;
        if addr < self.base || addr >= last {
            return false;
        }
        let idx = (addr - self.base) / PAGE_SIZE;
        self.bitmap
            .get(idx / 64)
            .is_some_and(|w| w & (1 << (idx % 64)) != 0)
    }
}

#[cfg(test)]
mod tests {
    // Tests link the harness's std; the lib itself stays no_std.
    extern crate std;

    use super::*;

    #[test]
    fn alloc_returns_aligned_addresses_in_first_fit_order() {
        let mut backing = [0u64; 1];
        let Ok(mut heap) = PageAlloc::new(0x1000, 8, &mut backing) else {
            // Infallible by construction (validated in unit tests).
            std::process::abort();
        };
        let Ok(a) = heap.alloc() else {
            // Infallible by construction (model guarantees a free slot).
            std::process::abort();
        };
        let Ok(b) = heap.alloc() else {
            // Infallible by construction (model guarantees a free slot).
            std::process::abort();
        };
        assert_eq!(a, 0x1000);
        assert_eq!(b, 0x2000);
        assert_eq!(a % PAGE_SIZE, 0);
        assert!(heap.is_allocated(a));
        assert!(heap.is_allocated(b));
        assert_eq!(heap.used_pages(), 2);
    }

    #[test]
    fn free_then_realloc_returns_same_page() {
        let mut backing = [0u64; 1];
        let Ok(mut heap) = PageAlloc::new(0x10_000, 4, &mut backing) else {
            // Infallible by construction (validated in unit tests).
            std::process::abort();
        };
        let Ok(a) = heap.alloc() else {
            // Infallible by construction (model guarantees a free slot).
            std::process::abort();
        };
        assert!(heap.free(a).is_ok(), "free: page is live");
        assert!(!heap.is_allocated(a));
        let Ok(b) = heap.alloc() else {
            // Infallible by construction (model guarantees a free slot).
            std::process::abort();
        };
        assert_eq!(a, b, "first-fit must hand back the lowest free page");
    }

    #[test]
    fn exhaustion_is_an_error_not_a_panic() {
        let mut backing = [0u64; 1];
        let Ok(mut heap) = PageAlloc::new(0x1000, 2, &mut backing) else {
            // Infallible by construction (validated in unit tests).
            std::process::abort();
        };
        assert!(heap.alloc().is_ok());
        assert!(heap.alloc().is_ok());
        assert_eq!(heap.alloc(), Err(HeapError::Exhausted));
    }

    #[test]
    fn double_free_is_detected() {
        let mut backing = [0u64; 1];
        let Ok(mut heap) = PageAlloc::new(0x1000, 2, &mut backing) else {
            // Infallible by construction (validated in unit tests).
            std::process::abort();
        };
        let Ok(a) = heap.alloc() else {
            // Infallible by construction (model guarantees a free slot).
            std::process::abort();
        };
        assert!(heap.free(a).is_ok(), "free: page is live");
        assert_eq!(heap.free(a), Err(HeapError::DoubleFree));
    }

    #[test]
    fn foreign_and_misaligned_frees_are_rejected() {
        let mut backing = [0u64; 1];
        let Ok(mut heap) = PageAlloc::new(0x1000, 2, &mut backing) else {
            // Infallible by construction (validated in unit tests).
            std::process::abort();
        };
        assert_eq!(heap.free(0x1000 + 8), Err(HeapError::NotAligned));
        assert_eq!(heap.free(0x500_000), Err(HeapError::NotOwned));
        assert!(heap.alloc().is_ok());
    }

    #[test]
    fn constructor_rejects_bad_layout() {
        let mut small = [0u64; 1];
        let mut big = [0u64; 2];
        // Misaligned base is rejected regardless of bitmap size.
        assert_eq!(
            PageAlloc::new(0x1001, 8, &mut small).err(),
            Some(HeapError::BadLayout)
        );
        assert_eq!(
            PageAlloc::new(0x1000, 0, &mut small).err(),
            Some(HeapError::BadLayout)
        );
        // 129 pages need 3 words; 2 words is too few.
        assert_eq!(
            PageAlloc::new(0x1000, 129, &mut big).err(),
            Some(HeapError::BadLayout)
        );
    }
}
