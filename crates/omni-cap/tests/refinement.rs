//! Phase-3 refinement oracle (ADR-0014 pattern): executes
//! `capability_contains_bound` (proofs/Proofs.lean) against
//! `HardwareCapability::contains`. A regression in the region check —
//! half-open edges, offset bound, overflow fail-closed — breaks these
//! tests, and with them `make ci`.

use omni_cap::HardwareCapability;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Executes `capability_contains_bound`: for `base <= addr <
    /// base + bound`, `contains(addr)` holds and the offset stays
    /// below `bound` (the lemma's conclusion). The edges are half-open
    /// — the far end and below-base are both rejected — and the
    /// overflow-fail-closed branch of `contains` is exercised
    /// separately by the deterministic test below.
    #[test]
    fn contains_refines_half_open_bound(
        base in any::<u64>(),
        bound in 1u64..,
        off in any::<u64>(),
    ) {
        prop_assume!(base.checked_add(bound).is_some());
        let cap = HardwareCapability::new(base, bound, 0, 0);
        let addr = base + off % bound;
        prop_assert!(cap.contains(addr), "in-region addr accepted");
        prop_assert!(addr - base < bound, "offset bounded by the lemma");
        prop_assert!(!cap.contains(base + bound), "far edge (half-open) rejected");
        prop_assert!(!cap.contains(base.wrapping_sub(1)), "below base rejected");
    }
}

/// The zero-bound edge: a capability granting nothing rejects every
/// address, including its own base (bound == 0 makes the region
/// empty).
#[test]
fn zero_bound_grants_nothing() {
    let cap = HardwareCapability::new(0x1000, 0, 0, 0);
    assert!(!cap.contains(0x1000), "empty region rejects its base");
    assert!(!cap.contains(0x1001));
    assert!(!cap.contains(0x0FFF));
}

/// The overflow-fail-closed contract (outside the Lean lemma, which
/// quantifies over non-overflowing bounds): `base + bound` past
/// `u64::MAX` must grant nothing, not wrap into a bogus region.
#[test]
fn overflowing_bound_fails_closed() {
    let cap = HardwareCapability::new(u64::MAX - 4, 8, 0, 0);
    assert!(
        !cap.contains(u64::MAX - 4),
        "wrapping region grants nothing, not even its base"
    );
    assert!(!cap.contains(u64::MAX));
    assert!(!cap.contains(0));
}

/// Executes `perm_pack_bounded` (`proofs/Proofs.lean`, goals §184
/// pattern): every defined permission bit sits inside the low byte, so
/// packing into a u8 — the MAC-canonical form — is lossless and the
/// reserved bits stay clear. Expressed in the u32 domain to keep the
/// mask arithmetic itself the subject (pack = `& 0xFF`).
#[test]
fn permission_bits_stay_in_low_byte() {
    let bits = [
        omni_cap::perms::READ,
        omni_cap::perms::WRITE,
        omni_cap::perms::EXEC,
        omni_cap::perms::DMA,
        omni_cap::perms::CXL,
    ];
    for bit in bits {
        assert_eq!(
            bit & 0xFF,
            bit,
            "perm bit packs losslessly into the low byte"
        );
    }
    assert_eq!(
        omni_cap::perms::ALL & 0xFF,
        omni_cap::perms::ALL,
        "the whole defined set packs losslessly"
    );
    assert_eq!(omni_cap::perms::ALL.count_ones(), 5, "five defined bits");
    // The composed token honors the same mask under grants_all.
    let cap = HardwareCapability::new(0, 0x1000, omni_cap::perms::ALL, 0);
    assert!(cap.grants_all(omni_cap::perms::ALL));
    assert!(cap.grants_all(0));
    assert!(!cap.grants_all(1 << 8), "a reserved bit is never granted");
}
