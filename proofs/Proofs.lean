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

#print axioms ring_index_bounded
#print axioms ring_mask_bounded
#print axioms perm_pack_bounded
#print axioms capability_contains_bound
