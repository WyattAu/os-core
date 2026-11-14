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

/-! ### omni-heap page layout (REQ-OS-007)

The allocator hands out `base + idx * PAGE_SIZE` for `idx < pages`
(first-fit, deterministic). These two contracts are what kernel
consumers rely on: every page is aligned for an aligned base, and
every page sits inside the managed range.
-/

theorem heap_page_aligned (base idx : Nat) (h : base % 4096 = 0) :
    (base + idx * 4096) % 4096 = 0 := by omega

theorem heap_page_in_range (idx pages : Nat) (h : idx < pages) :
    idx * 4096 < pages * 4096 := by omega

/-! ### Ring window injectivity (REQ-OS-001, Phase-3 first artifact)

The SPSC ring addresses slot `head & (CAP - 1)` for the `head`-th
push. FIFO correctness rests on a non-aliasing fact: two descriptors
inside one window of `CAP` can never share a slot. Theorems:

- `mod_window_injective`: the general-mod engine — two in-window sums
  with equal residues have equal summands.
- `ring_mask_injective`: the masked form the Rust ring actually uses
  (`x & (CAP - 1)`, `CAP = 2 ^ k`).
-/

theorem mod_window_injective (r j1 j2 n : Nat)
    (hr : r < n) (h1 : j1 < n) (h2 : j2 < n)
    (heq : (r + j1) % n = (r + j2) % n) : j1 = j2 := by
  by_cases hb1 : r + j1 < n
  · by_cases hb2 : r + j2 < n
    · have e1 : (r + j1) % n = r + j1 := Nat.mod_eq_of_lt hb1
      have e2 : (r + j2) % n = r + j2 := Nat.mod_eq_of_lt hb2
      rw [e1, e2] at heq
      omega
    · have hle : n ≤ r + j2 := by omega
      have e2 : (r + j2) % n = (r + j2 - n) % n := Nat.mod_eq_sub_mod hle
      have e2' : (r + j2 - n) % n = r + j2 - n :=
        Nat.mod_eq_of_lt (show r + j2 - n < n by omega)
      rw [e2, e2', Nat.mod_eq_of_lt hb1] at heq
      omega
  · by_cases hb2 : r + j2 < n
    · have hle : n ≤ r + j1 := by omega
      have e1 : (r + j1) % n = (r + j1 - n) % n := Nat.mod_eq_sub_mod hle
      have e1' : (r + j1 - n) % n = r + j1 - n :=
        Nat.mod_eq_of_lt (show r + j1 - n < n by omega)
      rw [e1, e1', Nat.mod_eq_of_lt hb2] at heq
      omega
    · have hle1 : n ≤ r + j1 := by omega
      have hle2 : n ≤ r + j2 := by omega
      have e1 : (r + j1) % n = (r + j1 - n) % n := Nat.mod_eq_sub_mod hle1
      have e2 : (r + j2) % n = (r + j2 - n) % n := Nat.mod_eq_sub_mod hle2
      have e1' : (r + j1 - n) % n = r + j1 - n :=
        Nat.mod_eq_of_lt (show r + j1 - n < n by omega)
      have e2' : (r + j2 - n) % n = r + j2 - n :=
        Nat.mod_eq_of_lt (show r + j2 - n < n by omega)
      rw [e1, e1', e2, e2'] at heq
      omega

/-- **Ring mask injectivity** (REQ-OS-001): the masked form the ring
uses — `slot = (head + j) & (CAP - 1)` with `CAP = 2 ^ k`. Two
descriptors in one window share a slot only when they are the same
descriptor, so FIFO order is slot-consistent. -/
theorem ring_mask_injective (head j1 j2 k : Nat)
    (h1 : j1 < 2 ^ k) (h2 : j2 < 2 ^ k)
    (heq : (head + j1) &&& (2 ^ k - 1) = (head + j2) &&& (2 ^ k - 1)) :
    j1 = j2 := by
  have expand1 : (head + j1) % 2 ^ k = (head % 2 ^ k + j1) % 2 ^ k := by
    rw [Nat.add_mod, Nat.mod_eq_of_lt h1]
  have expand2 : (head + j2) % 2 ^ k = (head % 2 ^ k + j2) % 2 ^ k := by
    rw [Nat.add_mod, Nat.mod_eq_of_lt h2]
  rw [Nat.and_two_pow_sub_one_eq_mod, Nat.and_two_pow_sub_one_eq_mod,
    expand1, expand2] at heq
  exact mod_window_injective (head % 2 ^ k) j1 j2 (2 ^ k)
    (Nat.mod_lt _ (Nat.two_pow_pos k)) h1 h2 heq

/-- **In-flight non-aliasing** (REQ-OS-001, Phase-3): two DISTINCT
in-flight descriptors (both inside the tail window of an in-flight
count no larger than CAP) cannot occupy the same slot — aliasing a
slot mid-flight would corrupt the descriptor being transported. Direct
contrapositive of the window injectivity engine. -/
theorem ring_inflight_distinct (tail j1 j2 k inflight : Nat)
    (h1 : j1 < inflight) (h2 : j2 < inflight) (hne : j1 ≠ j2)
    (hwindow : inflight ≤ 2 ^ k)
    (heq : (tail + j1) &&& (2 ^ k - 1) = (tail + j2) &&& (2 ^ k - 1)) :
    False := by
  have hj1 : j1 < 2 ^ k := Nat.lt_of_lt_of_le h1 hwindow
  have hj2 : j2 < 2 ^ k := Nat.lt_of_lt_of_le h2 hwindow
  exact hne (ring_mask_injective tail j1 j2 k hj1 hj2 heq)

#print axioms ring_inflight_distinct
#print axioms ring_index_bounded
#print axioms ring_mask_bounded
#print axioms perm_pack_bounded
#print axioms capability_contains_bound
#print axioms nvme_cqe_phase_is_high_half_lsb
#print axioms nvme_create_cq_cqid_extraction
#print axioms nvme_create_cq_qsize_extraction
#print axioms heap_page_aligned
#print axioms heap_page_in_range
#print axioms mod_window_injective
#print axioms ring_mask_injective
