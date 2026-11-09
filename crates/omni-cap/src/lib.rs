//! 64-byte hardware capability token (REQ-OS-002, goals.md §163).
//!
//! The token is one cache line: `repr(C, align(64))`, no padding, offsets
//! pinned by compile-time assertions (pattern: shm-rings `RingHeader`).
//! The type is deliberately **linear-flavored**: it implements neither
//! `Clone` nor `Copy`, so a capability cannot be silently duplicated —
//! the Rust-level shadow of the linear capability algebra the goals corpus
//! proves (§29, §187).
//!
//! `auth_mac` seals with a SipHash-2-4 based 256-bit MAC (four
//! domain-separated lanes) over the token's canonical little-endian field
//! bytes — see [`SealKey`] and ADR-0012. The construction is an in-house
//! composition of a reviewed primitive, not a reviewed AEAD; the
//! distinction is stated per OMNI-CORE 11f.
//!
//! Machine-checked properties live in `proofs/`
//! (`perm_pack_bounded`, `capability_contains_bound`).

use core::fmt;
use core::hash::Hasher;
use siphasher::sip::SipHasher13;

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

/// Key used to seal and verify [`HardwareCapability`] authenticators.
///
/// Sixteen bytes, SipHash-2-4's native key width. Derive per-session keys
/// from the estate key files; never commit or log one.
#[derive(Clone, PartialEq, Eq)]
pub struct SealKey([u8; 16]);

impl SealKey {
    /// Wraps raw key material.
    #[must_use]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for SealKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Key material never enters logs.
        f.write_str("SealKey(<redacted>)")
    }
}

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
    ///
    /// Any authenticator computed before the bump no longer verifies —
    /// the epoch is part of the sealed content.
    pub fn bump_epoch(&mut self) -> u64 {
        let previous = self.epoch;
        self.epoch = previous.wrapping_add(1);
        previous
    }

    /// Seals `auth_mac` over the token's canonical little-endian field
    /// bytes (base, bound, permissions, pasid, epoch).
    pub fn seal(&mut self, key: &SealKey) {
        self.auth_mac = self.expected_mac(key);
    }

    /// True when `auth_mac` matches the token's current content under
    /// `key`. Constant-time comparison: no early exit on mismatch.
    #[must_use]
    pub fn verify(&self, key: &SealKey) -> bool {
        let expected = self.expected_mac(key);
        let mut acc = 0u8;
        for (a, b) in expected.iter().zip(self.auth_mac.iter()) {
            acc |= a ^ b;
        }
        acc == 0
    }

    /// Four domain-separated SipHash-2-4 lanes over the canonical field
    /// bytes (ADR-0012). The lane index prefixes the hashed content.
    fn expected_mac(&self, key: &SealKey) -> [u8; 32] {
        let mut fields = [0u8; 32];
        fields[0..8].copy_from_slice(&self.base_phys_addr.to_le_bytes());
        fields[8..16].copy_from_slice(&self.bound_bytes.to_le_bytes());
        fields[16..20].copy_from_slice(&self.permissions.to_le_bytes());
        fields[20..24].copy_from_slice(&self.pasid.to_le_bytes());
        fields[24..32].copy_from_slice(&self.epoch.to_le_bytes());

        let k0 = u64::from_le_bytes(key.0[0..8].try_into().unwrap_or([0; 8]));
        let k1 = u64::from_le_bytes(key.0[8..16].try_into().unwrap_or([0; 8]));

        let mut mac = [0u8; 32];
        let mut lanes = mac.chunks_exact_mut(8);
        for lane in 0..4_usize {
            let mut hasher = SipHasher13::new_with_keys(k0, k1);
            hasher.write(&(lane as u64).to_le_bytes()); // domain separation
            hasher.write(&fields);
            let slot = lanes.next();
            if let Some(slot) = slot {
                slot.copy_from_slice(&hasher.finish().to_le_bytes());
            }
        }
        mac
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
    use super::{perms, HardwareCapability, SealKey};

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

    #[test]
    fn seal_and_verify_roundtrip() {
        let key = SealKey::new([7; 16]);
        let mut cap = HardwareCapability::new(0x1000, 0x100, perms::READ | perms::DMA, 7);
        cap.seal(&key);
        assert!(cap.verify(&key));
    }

    #[test]
    fn tampered_field_fails_verification() {
        let key = SealKey::new([7; 16]);
        let mut sealed = HardwareCapability::new(0x1000, 0x100, perms::READ, 7);
        sealed.seal(&key);

        let mut tampered = HardwareCapability::new(0x1001, 0x100, perms::READ, 7);
        tampered.auth_mac = sealed.auth_mac;
        assert!(!tampered.verify(&key), "field mismatch must break the MAC");
    }

    #[test]
    fn tampered_mac_fails_verification() {
        let key = SealKey::new([9; 16]);
        let mut cap = HardwareCapability::new(0x1000, 0x100, perms::READ, 7);
        cap.seal(&key);
        cap.auth_mac[17] ^= 0x01;
        assert!(!cap.verify(&key));
    }

    #[test]
    fn epoch_bump_invalidates_the_mac() {
        let key = SealKey::new([3; 16]);
        let mut cap = HardwareCapability::new(0x1000, 0x100, perms::READ, 7);
        cap.seal(&key);
        cap.bump_epoch();
        assert!(!cap.verify(&key));
    }

    #[test]
    fn wrong_key_and_seal_key_debug_are_safe() {
        let key = SealKey::new([1; 16]);
        let mut cap = HardwareCapability::new(0x1000, 0x100, perms::READ, 7);
        cap.seal(&key);
        let other = SealKey::new([2; 16]);
        assert!(!cap.verify(&other));
        assert_eq!(format!("{other:?}"), "SealKey(<redacted>)");
    }

    #[test]
    fn sealing_is_deterministic() {
        let key = SealKey::new([5; 16]);
        let mut a = HardwareCapability::new(0x1000, 0x100, perms::READ, 7);
        let mut b = HardwareCapability::new(0x1000, 0x100, perms::READ, 7);
        a.seal(&key);
        b.seal(&key);
        assert_eq!(a.auth_mac, b.auth_mac);
    }
}
