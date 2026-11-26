# Phase status — os-core against the corpus (ADR-0009)

- **Status**: point-in-time assessment
- **Date**: 2026-11-21
- **Source of truth**: [index.md](index.md) (keystones, phases, exit
  criteria) + [ADR-0009](../adr/0009-ten-year-goals-phasing.md) (the
  keystones-first strategy). This file is the dated snapshot; the
  index remains canonical.

## Where we are: Phase 1 exit met; standing at the Phase 1 -> 2 boundary

| Phase | Exit criteria (index.md) | Status | Evidence |
|---|---|---|---|
| 0 | spec the vision | **met** (2026-11-09) | ADR-0009 + the corpus split |
| 1 | omni-ring/omni-cap shipped; Lean gate green; Linux bypass spike | **met** (2026-11-20) | 11 published crate-versions (omni-ring 0.2, omni-cap, omni-heap, omni-task, omni-block, omni-sbe 0.1.1, omni-io, omni-tick 0.1/0.2, omni-md, omni-md-server); `make ci` carries the Lean gate (blocking) + Kani; A53 SQPOLL spike numbers |
| 2 | tick-to-trade demo on one board; microcore boots QEMU -> hardware | **in progress** | QEMU boot: done, gate-enforced (blockcheck witness). Hardware boot: attempted 2026-11-25 (Lenovo i5-7400 via Limine->multiboot2, read-only smoke; verdict pending screen/CMOS reading). Tick-to-trade: the data plane exists (omni-io -> omni-sbe -> omni-md -> omni-tick -> omni-md-server); the software demo runs at 33 ns p50 (256 ticks, 169 trades); the UDP feed socket landed 2026-11-26; no board run yet |
| 3 | microcore refinement proof; proof-carrying boot; DSL -> RTL | **rungs landed** | ADR-0014 ladder: mirrors + bitwise bridge + def-form theorems + cursor machine; Kani model-checking. The refinement proof proper (seL4-scale) remains the long pole |
| 4 | integrated heterogeneous stack | not entered | K5/CXL-dependent |

## The six keystones, current standing

| Keystone | Standing | Next artifact |
|---|---|---|
| K1 verified rings + capability algebra | proven + published + model-checked | MAC-sealing handshake doc only if a consumer emerges |
| K2 zero-bypass I/O | QEMU-proven (NVMe full path) + host io_uring seam published; UDP feed socket landed (2026-11-26) | real-metal NVMe smoke |
| K3 Lean proof gate | blocking in CI; executable mirrors; Kani pilot | rung D extraction (the decade pole) |
| K4 bare-metal microcore | QEMU boot + NVMe + gated task witness | hardware boot (see the two paths below) |
| K5 FPGA co-synthesis | parked | the board decision |
| K6 language | parked by design (DSL before compiler) | proc-macro DSL after Phase 3 |

## Next goals, ranked (per the corpus, not by preference)

1. **Microcore on real hardware** (Phase 2, K4). Two paths:
   - x86_64 on a dedicated fleet machine (USB/`kexec` boot; ADR-0009
     forbids daily-driver bets) — lowest new-code cost, proves the PVH
     boot + NVMe driver against real silicon;
   - aarch64 port for the A53/S7 — far larger (arch port, UART/GIC
     bring-up), but it is the board the HFT stack already targets.
   Decision needed: which board is "the one board".
2. **Tick-to-trade demo on one board** (Phase 2, hft pillars XXII–XXVI).
   The SOFTWARE form is done (2026-11-21): omni-md's
   `tick_to_trade` example runs the whole plane per tick — SBE
   encode → decode → strategy decide → segment persist — at 33 ns
   min / 36 ns p50 (release), 169 trades over 256 ticks. The BOARD
   form (real feed socket + the A53) is the remaining half.
3. **Server/database first designs** (store/db pillars XLVII–LVI) —
   started: ADR-0017 + omni-tick are pillar LIV's software form
   (521 lock-free tick log; 530 monotonic invariant; 526 epoch/range
   queries this tranche). Queued from LIV: OHLCV
   tick-to-bar synthesis (528). Gorilla XOR compression (522) landed
   2026-11-21 as `omni_tick::gorilla` (software form). LII/LIII (query engines,
   HTAP) queue behind the store. XLIX (NVMe ZNS) queues behind
   real-metal NVMe.
4. **Phase 3 refinement rungs** (K3/K4) — cursor machine formalized;
   next: drive the mirror defs into the kernel's cursor code paths and
   the Kani-vs-Lean cross-check.

## Explicitly parked (per ADR-0009)

- K5 FPGA work: the board decision. Nothing FPGA-shaped starts before
  it.
- K6 language: proc-macro DSL is a Phase-3 artifact.
- Silicon-dependent items: specified or proved as Lean specs only
  (no tape-outs).

## What this file is not

A promise. Exit criteria are met when their evidence survives
`make ci` and (for Phase 2) a hardware run — not when this table says
so.

## Dated log

### 2026-11-26 — the OTA update loop is proven (keyboard-free path)

- **T4b closed**: the omniosk kernel now detects a staged update
  (LBAs 34+), validates the ELF, drops through the hand-encoded 32-bit
  trampoline (per-segment copy + dword tail + bss zero-fill), and
  re-enters itself as the update-loaded hardware profile (boot_kind 3).
  Proof standard: ADR-0014's oracle policy — `scripts/qemu-ota.sh`
  boots the gate with the payload's banner string patched, so the
  second boot printing the patched string is evidence the STAGED image
  executed (not a reboot). Gate wired into `make ci` (4f105bf).
- The gate caught six bugs pre-push, two of them latent real-hardware
  defects: the NVMe SQ doorbell never wrapped (every 64th I/O hung),
  and the boot asm's bss-zero read its count from video-RAM garbage
  (masked in QEMU by zeroed RAM; would have corrupted forward memory
  on real silicon).
- **omni-md real feed socket landed** (42fe8fd): one SBE frame per
  UDP datagram decodes through the same pipeline seam as the synthetic
  seed; `/stats` carries udp_in/udp_ticks/udp_bad. The software half
  of the Phase-2 tick-to-trade demo's "real feed" is now a socket, not
  a placeholder.
- **Blocked on one physical action**: the Lenovo boots the pre-loader
  OMNIOSK (192.168.1.191 unreachable). One Limine menu pick restores
  CachyOS/SSH; then the new ELF deploys over SSH and every further
  update is keyboard-free through the now-verified OTA path.

- **The Limine boot path is now CI-gated** (e2888de): `scripts/qemu-limine.sh`
  boots the deployment artifact through the server's actual Limine 12.9.6
  under OVMF and asserts the multiboot2 smoke. It caught three latent
  defects the PVH gates could never see: (1) the mb2 header tags were
  mis-assembled (`.short` pairs shifted the size field; the info-request
  tag lacked its 4-byte pad, so Limine's ALIGN_UP(8) stride skipped the
  entry-address tag); (2) the boot identity map filled only `pml4[0]` —
  OVMF parks the NVMe BAR at 0xC0_00000000 (768 GiB, PML4 index 1), so
  the first MMIO read triple-faulted (not-present, IDT=0); the map now
  covers [0, 1 TiB) via two PDPTs with both dwords written; (3) the NVMe
  BAR read took only the low dword, so 64-bit BARs read as 0.
- **Exit protocol changed: halt-forever -> 8042 reset** (d4cf51e): the
  firmware paths (boot_kind 2/3) now print their verdict, drain the
  UART, and pulse the 8042 reset line — the machine returns through the
  bootloader to the default OS and ssh comes back. Combined with UDP
  checkpoint beacons (NIC probed before the smoke: enter / nvme ok/fail
  / full verdict), a hardware smoke is fully remotely observable and
  the boot loop is keyboard-free end to end. 2026-11-26 screen
  evidence: after Limine's "multiboot2 executable boot():/omniosk.elf"
  line the screen stays frozen by design (the kernel talks COM1 only) —
  a halted smoke and a hang are indistinguishable on the panel, which
  is exactly what the beacons eliminate.
- **Remaining physical action**: one CachyOS pick to restore SSH; then
  deploy d4cf51e, flip UEFI BootOrder to CachyOS-default with Limine on
  `efibootmgr --bootnext`, and every future hardware smoke is one
  remote command.
