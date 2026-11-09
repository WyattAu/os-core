//! 64-byte hardware capability token (REQ-OS-002, goals.md §163).
//!
//! The token is one cache line: `repr(C, align(64))`, no padding, offsets
//! pinned by compile-time assertions (pattern: shm-rings `RingHeader`).
//! The type is deliberately **linear-flavored**: it implements neither
//! `Clone` nor `Copy`, so a capability cannot be silently duplicated —
//! the Rust-level shadow of the linear capability algebra the goals corpus
//! proves (§29, §187).
//!
//! `auth_mac` is an opaque 32-byte field for an external authenticator;
//! MAC *computation* is intentionally out of scope here (no hand-rolled
//! crypto) and tracked as follow-up behind a reviewed hashing crate.
//!
//! Machine-checked properties live in `proofs/`
//! (`perm_pack_bounded`, `capability_contains_bound`).

use core::fmt;

/// Permission bits valid in [`HardwareCapability::permissions`].
pub mod perms {
    /// Read access.
    pub const READ: u32 = 1 << 0;
    /// Write access.
    pub const WRITE: u32 = 1 << 1;
    /// Execute access.
    pub const EXEC: u32 = 1 << 2;
    /// DMA target access.
    pub const DMA: u32 = 1 << 3;
    /// CXL fabric access.
    pub const CXL: u32 = 1 << 4;
    /// Every permission bit this crate defines.
    pub const ALL: u32 = READ | WRITE | EXEC | DMA | CXL;
}

/// A 64-byte capability token: base, bound, permissions, PASID, epoch,
/// and an opaque authenticator.
#[repr(C, align(64))]
pub struct HardwareCapability {
    /// Inclusive physical base address of the granted region.
    pub base_phys_addr: u64,
    /// Exclusive length of the granted region in bytes.
    pub bound_bytes: u64,
    /// [`perms`] bit set.
    pub permissions: u32,
    /// Hardware process address-space identifier.
    pub pasid: u32,
    /// Allocation generation counter; bumped on re-grant.
    pub epoch: u64,
    /// Opaque 32-byte authenticator (sealed externally; not interpreted).
    pub auth_mac: [u8; 32],
}

const _: () = assert!(size_of::<HardwareCapability>() == 64);
const _: () = assert!(align_of::<HardwareCapability>() == 64);
// repr(C) fixes declaration order; these pin the layout contract so a
// hardware table or DMA descriptor can be cast onto this type safely.
const _: () = assert!(core::mem::offset_of!(HardwareCapability, base_phys_addr) == 0);
const _: () = assert!(core::mem::offset_of!(HardwareCapability, bound_bytes) == 8);
const _: () = assert!(core::mem::offset_of!(HardwareCapability, permissions) == 16);
const _: () = assert!(core::mem::offset_of!(HardwareCapability, pasid) == 20);
const _: () = assert!(core::mem::offset_of!(HardwareCapability, epoch) == 24);
const _: () = assert!(core::mem::offset_of!(HardwareCapability, auth_mac) == 32);

impl HardwareCapability {
    /// Grants `[base, base + bound)` to `pasid` with `permissions`.
    ///
    /// Epoch starts at zero and the authenticator starts zeroed; sealing is
    /// an external step by design.
    #[must_use]
    pub fn new(base_phys_addr: u64, bound_bytes: u64, permissions: u32, pasid: u32) -> Self {
        Self {
            base_phys_addr,
            bound_bytes,
            permissions,
            pasid,
            epoch: 0,
            auth_mac: [0; 32],
        }
    }

    /// True when `permissions` covers every bit in `needed`.
    #[must_use]
    pub fn grants_all(&self, needed: u32) -> bool {
        self.permissions & needed == needed
    }

    /// True when `addr` lies in the granted region.
    ///
    /// Overflow-fail-closed: if `base + bound` exceeds `u64::MAX`, the
    /// capability grants nothing (the Lean lemma
    /// `capability_contains_bound` covers the non-overflow case; overflow
    /// rejects here rather than wrapping).
    #[must_use]
    pub fn contains(&self, addr: u64) -> bool {
        let Some(end) = self.base_phys_addr.checked_add(self.bound_bytes) else {
            return false;
        };
        addr >= self.base_phys_addr && addr < end
    }

    /// Bumps the allocation generation, returning the previous epoch.
    pub fn bump_epoch(&mut self) -> u64 {
        let previous = self.epoch;
        self.epoch = previous.wrapping_add(1);
        previous
    }
}

impl fmt::Debug for HardwareCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `auth_mac` is deliberately omitted: capabilities debug-prints must
        // not leak authenticator material into logs.
        f.debug_struct("HardwareCapability")
            .field("base_phys_addr", &self.base_phys_addr)
            .field("bound_bytes", &self.bound_bytes)
            .field("permissions", &self.permissions)
            .field("pasid", &self.pasid)
            .field("epoch", &self.epoch)
            .field("auth_mac", &"<sealed>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{perms, HardwareCapability};

    #[test]
    fn layout_contract_is_one_cache_line() {
        assert_eq!(size_of::<HardwareCapability>(), 64);
        assert_eq!(align_of::<HardwareCapability>(), 64);
    }

    #[test]
    fn contains_is_half_open() {
        let cap = HardwareCapability::new(0x1000, 0x100, perms::READ | perms::DMA, 7);
        assert!(!cap.contains(0x0FFF));
        assert!(cap.contains(0x1000));
        assert!(cap.contains(0x10FF));
        assert!(!cap.contains(0x1100));
    }

    #[test]
    fn contains_fails_closed_on_end_overflow() {
        let cap = HardwareCapability::new(u64::MAX - 1, 10, perms::READ, 1);
        assert!(!cap.contains(u64::MAX));
    }

    #[test]
    fn grants_checks_whole_mask() {
        let cap = HardwareCapability::new(0, 1, perms::READ | perms::WRITE, 1);
        assert!(cap.grants_all(perms::READ));
        assert!(cap.grants_all(perms::READ | perms::WRITE));
        assert!(!cap.grants_all(perms::READ | perms::EXEC));
    }

    #[test]
    fn epoch_is_monotonic_and_previous_returned() {
        let mut cap = HardwareCapability::new(0, 1, perms::READ, 1);
        assert_eq!(cap.bump_epoch(), 0);
        assert_eq!(cap.bump_epoch(), 1);
    }

    #[test]
    fn token_is_linear_no_clone() {
        // Compile-time statement of linearity: moves are fine, duplication
        // is impossible. (No Clone/Copy impls exist to call.)
        let cap = HardwareCapability::new(0, 1, perms::READ, 1);
        let moved = cap;
        assert_eq!(moved.pasid, 1);
    }

    #[test]
    fn debug_renders_fields_but_seals_authenticator() {
        let mut cap = HardwareCapability::new(0x2000, 0x10, perms::READ, 3);
        cap.auth_mac[0] = 0xAB;
        let rendered = format!("{cap:?}");
        assert!(rendered.contains("base_phys_addr: 8192"));
        assert!(rendered.contains("epoch: 0"));
        assert!(rendered.contains("<sealed>"));
        // 0xAB = 171 decimal must not leak through the debug render.
        assert!(!rendered.contains("171"));
    }
}
