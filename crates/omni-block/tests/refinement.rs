//! Phase-3 refinement oracle (ADR-0014 pattern): executes the `NVMe`
//! wire-layout theorems (`proofs/Proofs.lean`, REQ-OS-006) against the
//! `wire` helpers the kernel consumes. A byte-order regression —
//! reading the phase from bit 0, writing a 1-based QSIZE, splitting
//! CDW10 wrongly — breaks these tests, and with them `make ci`.

use omni_block::wire;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Executes `nvme_cqe_phase_is_high_half_lsb`: for DW3 composed as
    /// `cid | status << 16`, `cqe_phase_of(dw3) == status & 1` for all
    /// 16-bit halves.
    #[test]
    fn cqe_phase_is_high_half_lsb(cid in 0u16.., status in 0u16..) {
        let dw3 = u32::from(cid) | (u32::from(status) << 16);
        prop_assert_eq!(
            wire::cqe_phase_of(dw3),
            u32::from(status & 1),
            "phase is DW3 bit 16 (status LSB)"
        );
        prop_assert_eq!(wire::cqe_status(dw3), status, "status half intact");
        prop_assert_eq!(wire::cqe_cid(dw3), cid, "cid half intact");
    }

    /// Executes `nvme_create_cq_cqid_extraction` +
    /// `nvme_create_cq_qsize_extraction`: CDW10 round-trips both
    /// halves exactly.
    #[test]
    fn create_q_cdw10_round_trips(cqid in 0u16.., qsize0 in 0u16..) {
        let cdw10 = wire::create_q_cdw10(cqid, qsize0);
        prop_assert_eq!(cdw10 & 0xFFFF, u32::from(cqid), "cqid extraction");
        prop_assert_eq!(cdw10 >> 16, u32::from(qsize0), "qsize extraction");
    }
}

/// The regression that started the theorem: reading the phase from
/// DW3 bit 0 (the CID's LSB) instead of bit 16. For identify
/// (CID = 1, odd) the wrong bit reads 1 and CQE polling appears to
/// complete; for create-cq (CID = 2, even) it never does. The correct
/// extraction disagrees with the wrong one exactly when CID's LSB and
/// the phase differ — this pins one such case on each side.
#[test]
fn wrong_bit_read_is_distinguishable() {
    // identify: CID = 1 (LSB 1), first poll phase = 0.
    let dw3 = u32::from(1u16); // status = 0 => phase 0
    assert_eq!(wire::cqe_phase_of(dw3), 0, "correct: status LSB");
    assert_eq!(
        dw3 & 1,
        1,
        "wrong bit would read the CID LSB (the 66e7d4b bug)"
    );

    // create-cq: CID = 2 (LSB 0), first poll phase = 1.
    let dw3 = u32::from(2u16) | (1u32 << 16);
    assert_eq!(wire::cqe_phase_of(dw3), 1, "correct: status LSB");
    assert_eq!(dw3 & 1, 0, "wrong bit would read 0 and never advance");
}
