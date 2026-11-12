import Mathlib.Data.Nat.Bitwise

/-!
# OsCoreProofs — machine-checked properties for os-core primitives (REQ-OS-002)

Each theorem corresponds to a property the Rust code relies on; the
correspondence is structural (same masking / containment arithmetic), not
yet extraction-based (goals.md §48 is a later milestone). OMNI-CORE 11f:
`sorry` fails the build; `#print axioms` is stated per theorem.
-/

/-- **Ring index bounding** (goals.md §181): bitwise-AND indexing on a
power-of-two capacity can never produce an out-of-bounds slot index. -/
theorem ring_index_bounded (head k : Nat) (hk : 0 < k) :
    head &&& (2 ^ k - 1) < 2 ^ k := by
  rw [Nat.and_two_pow_sub_one_eq_mod]
  exact Nat.mod_lt head (Nat.two_pow_pos k)

/-- **Masked indexing against an explicit capacity** (goals.md §162): with
`cap = 2^k`, `i &&& (cap - 1)` is a valid slot index. -/
theorem ring_mask_bounded (cap k i : Nat) (hc : cap = 2 ^ k) (hk : 0 < k) :
    i &&& (cap - 1) < cap := by
  subst hc
  exact ring_index_bounded i k hk

/-- **Permission packing** (goals.md §184 pattern): packing permissions
into the low byte leaves the value within the valid mask — reserved bits
stay clear. -/
theorem perm_pack_bounded (p : Nat) : p &&& 0xFF ≤ 0xFF := by
  have h : 0xFF = 2 ^ 8 - 1 := rfl
  rw [h, Nat.and_two_pow_sub_one_eq_mod]
  have hl := Nat.mod_lt p (Nat.two_pow_pos 8)
  simp only [show ((2:Nat) ^ 8) = 256 from rfl] at hl ⊢
  omega

/-- **Capability containment** (REQ-OS-002): an address in the half-open
range `[base, base + bound)` indexes strictly within the bound. -/
theorem capability_contains_bound (base bound addr : Nat)
    (hlo : base ≤ addr) (hhi : addr < base + bound) :
    addr - base < bound := by omega

/-! ### NVMe wire layout (REQ-OS-006)

Two field-order bugs were found empirically in the omniosk NVMe driver
(commit 66e7d4b): the CQE phase bit lives in the *high* half of DW3
(bytes 14-15), not bit 0 of the CID half; and the CreateCq/CreateSq
overlays split CDW10 as ID[15:0] | QSIZE[31:16]. These theorems encode
those contracts so the byte-order class of bug stays machine-checked.
-/

/-- **CQE phase placement** (Linux `nvme_completion`, QEMU `NvmeCqe`):
DW3 is the little-endian composition `CID | status << 16` where status
`= SF << 1 | P`. Extracting DW3's high half and masking bit 0 therefore
yields the status's LSB — the phase. (Here `status` is the composed
high half, byte 14 + byte 15 * 256, so its LSB is byte 14's LSB.)
Reading bit 0 of DW3 instead reads the CID's LSB, which is why odd-CID
commands (identify, CID = 1) appeared to complete while even-CID
commands (create-cq, CID = 2) never did. -/
theorem nvme_cqe_phase_is_high_half_lsb (cid status : Nat)
    (hc : cid < 65536) (hs : status < 65536) :
    (cid + status * 65536) >>> 16 &&& 1 = status % 2 := by
  have hshift : (cid + status * 65536) >>> 16 = status := by
    rw [Nat.shiftRight_eq_div_pow,
      show ((2 : Nat) ^ 16) = 65536 from rfl]
    have hmod : (cid + status * 65536) % 65536 = cid := by omega
    have hdam := Nat.div_add_mod (cid + status * 65536) 65536
    rw [hmod] at hdam
    have hstep : (cid + status * 65536) / 65536 * 65536
        = status * 65536 := by omega
    exact Nat.eq_of_mul_eq_mul_right
      (show (0 : Nat) < 65536 from by omega) hstep
  rw [hshift]
  have hmask : (1 : Nat) = 2 ^ 1 - 1 := rfl
  rw [hmask, Nat.and_two_pow_sub_one_eq_mod,
    show ((2 : Nat) ^ 1) = 2 from rfl]

/-- **CreateCq CDW10 low half** (QEMU `NvmeCreateCq` overlay, REQ-OS-006):
CQID occupies CDW10[15:0] — masking CDW10 with a 16-bit mask extracts
it exactly. -/
theorem nvme_create_cq_cqid_extraction (cqid qsize : Nat) (hc : cqid < 65536) :
    (cqid + qsize * 65536) &&& 65535 = cqid := by
  have hmask : 65535 = 2 ^ 16 - 1 := rfl
  rw [hmask, Nat.and_two_pow_sub_one_eq_mod,
    show ((2 : Nat) ^ 16) = 65536 from rfl]
  omega

/-- **CreateCq CDW10 high half** (QEMU `NvmeCreateCq` overlay,
REQ-OS-006): QSIZE occupies CDW10[31:16] — the 0-based entry count
(quantity - 1) must be written there, not as a 32-bit value spanning
the CQID field. Writing QSIZE-1 = 3 across CDW10[31:0] was the stale
trace `cqid=3, qsize=0` that cost a session. -/
theorem nvme_create_cq_qsize_extraction (cqid qsize : Nat)
    (hc : cqid < 65536) (hq : qsize < 65536) :
    (cqid + qsize * 65536) >>> 16 = qsize := by
  rw [Nat.shiftRight_eq_div_pow,
    show ((2 : Nat) ^ 16) = 65536 from rfl]
  have hmod : (cqid + qsize * 65536) % 65536 = cqid := by omega
  have hdam := Nat.div_add_mod (cqid + qsize * 65536) 65536
  rw [hmod] at hdam
  have hstep : (cqid + qsize * 65536) / 65536 * 65536
      = qsize * 65536 := by omega
  exact Nat.eq_of_mul_eq_mul_right
    (show (0 : Nat) < 65536 from by omega) hstep

#print axioms ring_index_bounded
#print axioms ring_mask_bounded
#print axioms perm_pack_bounded
#print axioms capability_contains_bound
#print axioms nvme_cqe_phase_is_high_half_lsb
#print axioms nvme_create_cq_cqid_extraction
#print axioms nvme_create_cq_qsize_extraction
