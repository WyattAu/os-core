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

/-- **Ring window injectivity** (REQ-OS-001, Phase-3): masked
descriptor addressing is injective within one window — two descriptors
`h1 + j1`, `h1 + j2` with distinct `j` (both below the capacity)
occupy distinct slots. This is the anti-aliasing guarantee that makes
FIFO order meaningful: a value written into descriptor `d` cannot be
overwritten by a write to another in-flight descriptor, so pop order =
push order. Generalizes the admin-CQE window analysis. -/
theorem ring_window_injective (h1 cap j1 j2 : Nat)
    (hc : 0 < cap) (hj1 : j1 < cap) (hj2 : j2 < cap)
    (heq : (h1 + j1) % cap = (h1 + j2) % cap) :
    j1 = j2 := by
  have e1 : (h1 + j1) % cap = (h1 % cap + j1) % cap := by
    rw [Nat.add_mod, Nat.mod_eq_of_lt hj1]
  have e2 : (h1 + j2) % cap = (h1 % cap + j2) % cap := by
    rw [Nat.add_mod, Nat.mod_eq_of_lt hj2]
  rw [e1, e2] at heq
  exact mod_window_injective (h1 % cap) j1 j2 cap
    (Nat.mod_lt _ hc) hj1 hj2 heq

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
#print axioms ring_inflight_distinct
#print axioms ring_window_injective

/-! ### Executable mirrors (ADR-0014 §48 rung)

Each `def` is the Lean form of a Rust helper the crates ship
(`omni_block::wire`, `SpscRing` slot arithmetic, `PageAlloc`,
`HardwareCapability::contains`); each mirror theorem pins its def to a
property proven above, and the `decide` vectors sample the same cases
the Rust oracles assert (`crates/*/tests/refinement.rs`). The spec
side is now executable: a Rust-side arithmetic change and a Lean-side
def change must agree or a gate breaks on at least one side.

Note: the bitwise/additive bridge is proven (`or_shift16_eq_add`
below), so `wire_create_q_cdw10` carries the Rust `|`/`<<` body
directly — no additive detour.
-/

/-- Mirror of `omni_block::wire::cqe_phase_of`: `(dw3 >> 16) & 1`. -/
def wire_cqe_phase (dw3 : Nat) : Nat := (dw3 >>> 16) &&& 1

/-- Mirror theorem: the executable def satisfies the CQE phase
placement contract. -/
theorem wire_cqe_phase_mirror (cid status : Nat)
    (hc : cid < 65536) (hs : status < 65536) :
    wire_cqe_phase (cid + status * 65536) = status % 2 :=
  nvme_cqe_phase_is_high_half_lsb cid status hc hs

/-- Mirror of `omni_block::wire::create_q_cdw10`: `cqid | (qsize0 <<< 16)` —
the line-for-line form of the Rust u32 body; the bitwise/additive
bridge below carries the wire theorems onto it. -/
def wire_create_q_cdw10 (cqid qsize0 : Nat) : Nat := cqid ||| (qsize0 <<< 16)

/-- Mirror of `SpscRing`'s slot index: `pos & (CAP - 1)`. The
def-form theorems below are the THEOREMS OF RECORD for the ring
surface (ADR-0014 rung C): the arithmetic forms above are the engines
that prove them, and the Rust oracles execute the def forms. -/
def spsc_slot (pos cap : Nat) : Nat := pos &&& (cap - 1)

/-- Theorem of record: the def's output is a valid slot index. -/
theorem spsc_slot_bounded (pos k : Nat) (hk : 0 < k) :
    spsc_slot pos (2 ^ k) < 2 ^ k :=
  ring_mask_bounded (2 ^ k) k pos rfl hk

/-- Theorem of record: distinct in-window descriptors never collide on
a slot (the FIFO-order guarantee, at the def). -/
theorem spsc_slot_injective (head j1 j2 k : Nat)
    (h1 : j1 < 2 ^ k) (h2 : j2 < 2 ^ k)
    (heq : spsc_slot (head + j1) (2 ^ k) = spsc_slot (head + j2) (2 ^ k)) :
    j1 = j2 :=
  ring_mask_injective head j1 j2 k h1 h2 heq

/-- Theorem of record: two DISTINCT in-flight descriptors (both inside
the tail window of an in-flight count no larger than the capacity)
occupy different slots. -/
theorem spsc_inflight_distinct (tail j1 j2 k inflight : Nat)
    (h1 : j1 < inflight) (h2 : j2 < inflight) (hne : j1 ≠ j2)
    (hwindow : inflight ≤ 2 ^ k) :
    spsc_slot (tail + j1) (2 ^ k) ≠ spsc_slot (tail + j2) (2 ^ k) :=
  ring_inflight_distinct tail j1 j2 k inflight h1 h2 hne hwindow

/-- Mirror of `PageAlloc`'s page address: `base + idx * PAGE_SIZE`. -/
def heap_page (base idx : Nat) : Nat := base + idx * 4096

/-- Mirror theorem: pages stay 4096-aligned for an aligned base. -/
theorem heap_page_aligned_mirror (base idx : Nat) (h : base % 4096 = 0) :
    heap_page base idx % 4096 = 0 :=
  heap_page_aligned base idx h

/-- Mirror theorem: the idx-th page sits below the idx-pages-th. -/
theorem heap_page_range_mirror (base idx pages : Nat) (h : idx < pages) :
    heap_page base idx < heap_page base pages := by
  simp only [heap_page]
  omega

/-- Mirror of `HardwareCapability::contains` (the non-overflow region
the Rust fn's fail-closed branch sits outside of). -/
abbrev cap_contains (base bound addr : Nat) : Prop := base ≤ addr ∧ addr < base + bound

/-- Mirror theorem: containment bounds the offset. -/
theorem cap_contains_mirror (base bound addr : Nat)
    (h : cap_contains base bound addr) : addr - base < bound :=
  capability_contains_bound base bound addr h.1 h.2

/- The bitwise/additive bridge (the §48 gap, closed here): on disjoint
16-bit halves, the wire `|`/`<<` composition IS the additive one. -/
theorem cdw10_low_bitwise (a b : Nat) (ha : a < 65536) (hb : b < 65536) :
    (a ||| (b <<< 16)) &&& 65535 = a := by
  have hmask : (65535 : Nat) = 2 ^ 16 - 1 := rfl
  rw [hmask, Nat.and_or_distrib_right]
  have hsh : (b <<< 16) &&& (2 ^ 16 - 1) = 0 := by
    rw [Nat.shiftLeft_eq, Nat.and_two_pow_sub_one_eq_mod]
    omega
  rw [hsh, Nat.or_zero, Nat.and_two_pow_sub_one_eq_mod, Nat.mod_eq_of_lt ha]

theorem cdw10_high_bitwise (a b : Nat) (ha : a < 65536) (hb : b < 65536) :
    (a ||| (b <<< 16)) >>> 16 = b := by
  apply Nat.eq_of_testBit_eq
  intro i
  rw [Nat.testBit_shiftRight, Nat.testBit_or, Nat.testBit_shiftLeft]
  have ha16 : Nat.testBit a (16 + i) = false := by
    have h0 : a >>> 16 = 0 := by
      rw [Nat.shiftRight_eq_div_pow, show ((2 : Nat) ^ 16) = 65536 from rfl]
      omega
    have ht : Nat.testBit a (16 + i) = Nat.testBit (a >>> 16) i :=
      (Nat.testBit_shiftRight a).symm
    rw [ht, h0]
    exact Nat.zero_testBit i
  rw [ha16]
  simp

theorem or_shift16_eq_add (a b : Nat) (ha : a < 65536) (hb : b < 65536) :
    a ||| (b <<< 16) = a + b * 65536 := by
  have hmask : (65535 : Nat) = 2 ^ 16 - 1 := rfl
  have hlow := cdw10_low_bitwise a b ha hb
  have hhigh := cdw10_high_bitwise a b ha hb
  rw [hmask, Nat.and_two_pow_sub_one_eq_mod] at hlow
  rw [Nat.shiftRight_eq_div_pow, show ((2 : Nat) ^ 16) = 65536 from rfl] at hhigh
  have hsplit := Nat.div_add_mod (a ||| (b <<< 16)) 65536
  rw [hhigh, hlow] at hsplit
  omega

/-- Mirror theorem: the CDW10 low half extracts the queue id (bitwise
form; the bounds are the bridge's disjointness hypotheses). -/
theorem wire_cdw10_cqid_mirror (cqid qsize0 : Nat)
    (hc : cqid < 65536) (hq : qsize0 < 65536) :
    wire_create_q_cdw10 cqid qsize0 &&& 65535 = cqid :=
  cdw10_low_bitwise cqid qsize0 hc hq

/-- Mirror theorem: the CDW10 high half extracts the 0-based queue
size. -/
theorem wire_cdw10_qsize_mirror (cqid qsize0 : Nat)
    (hc : cqid < 65536) (hq : qsize0 < 65536) :
    wire_create_q_cdw10 cqid qsize0 >>> 16 = qsize0 :=
  cdw10_high_bitwise cqid qsize0 hc hq

#print axioms or_shift16_eq_add
#print axioms cdw10_low_bitwise
#print axioms cdw10_high_bitwise

/- Wire vectors sampled from the Rust oracles: each `decide` below
matches an asserted case in `crates/*/tests/refinement.rs`. -/
example : wire_cqe_phase (1 + 0 * 65536) = 0 := by decide
example : wire_cqe_phase (2 + 1 * 65536) = 1 := by decide
example : wire_create_q_cdw10 1 63 &&& 65535 = 1 := by decide
example : wire_create_q_cdw10 1 63 >>> 16 = 63 := by decide
example : spsc_slot 9 8 = 1 := by decide
example : spsc_slot 8 8 = 0 := by decide
example : heap_page 65536 3 = 77824 := by decide
example : cap_contains 0 8 7 := by decide
example : ¬cap_contains 0 8 8 := by decide

#print axioms wire_cqe_phase_mirror
#print axioms wire_cdw10_cqid_mirror
#print axioms wire_cdw10_qsize_mirror
#print axioms spsc_slot_bounded
#print axioms spsc_slot_injective
#print axioms heap_page_aligned_mirror
#print axioms spsc_inflight_distinct
#print axioms heap_page_range_mirror
#print axioms cap_contains_mirror
