# ADR-0019: Firmware-path exit protocol — 8042 reset + UDP beacon telemetry

## Status

Accepted (2026-11-26)

## Context

The phase-2 hardware feedback loop ran on the Lenovo i5-7400
(192.168.1.191) with two structural observability failures:

1. **Terminal state was hlt-forever.** `machine_exit` on firmware
   paths (boot_kind 2 multiboot2, boot_kind 3 update-loaded) printed
   a halt line and looped on `hlt`. The kernel talks only on COM1 —
   the panel stays frozen at Limine's `multiboot2 executable
   boot():/omniosk.elf` line forever. A completed smoke, a hang, and a
   triple fault are indistinguishable on the screen, and ssh never
   returns: every later deploy needed a physical power-cycle plus a
   manual bootloader pick (the keyboard dependence the OTA loop exists
   to remove).
2. **The verdict datagram was emitted inside the success path.** The
   rtl8168 probe and UDP report ran only under
   `if let Some(nvme_session)` — any earlier failure (NIC absent,
   NVMe smoke failed) left the operator PC with zero packets.

## Decision

- `machine_exit` on firmware paths: print the verdict line, drain the
  UART (~8M spin cycles), then pulse the 8042 keyboard-controller
  reset line (`outb(0x64, 0xFE)`) in a loop. No firmware cooperation
  required; every x86 board honors it. The machine returns through
  the bootloader to the default OS and ssh comes back.
- The NIC is probed before the NVMe smoke. Checkpoint beacons go to
  the operator PC (192.168.1.1:9933): `enter` (NIC ok), `nvme session
  ok|FAILED`, then the full verdict report reusing the same `Nic`.
- Deployment topology: UEFI `BootOrder` defaults to CachyOS; Limine
  runs only via `efibootmgr --bootnext`. A hardware smoke is one
  remote command (`bootnext Limine && reboot`) and the machine always
  returns.

## Consequences

- The boot loop is keyboard-free end to end: reboot → OMNIOSK smoke +
  beacons → reset → CachyOS → ssh.
- The QEMU gates run with `-no-reboot`: the guest reset exits QEMU
  cleanly (status 0). The OTA gate's old run-to-timeout heuristic
  (status 124) was replaced by asserting the verdict line in the
  serial log — an early exit without the line still fails the gate
  (triple fault).
- A smoke run now costs one extra full POST cycle (reset → CachyOS);
  acceptable at operator cadence.
- The kernel cannot restore `limine.conf` after editing it (no FAT
  writer by design); the BootNext topology removes that need.
- hlt-forever remains for nothing: every terminal state now cycles.

## Alternatives considered

- **Serial console on the Lenovo** — needs a physical COM cable and
  screen/serial reading, violating the no-screen-reading gate policy.
- **Kernel writes `limine.conf` back to CachyOS-default before
  reset** — requires a FAT writer in the kernel; large attack/maint
  surface for a config flip.
- **Reboot into OMNIOSK every power cycle (current BootOrder)** —
  every test ends dark; operator must babysit the menu each run.
- **kexec into CachyOS from the kernel** — needs a preserved CachyOS
  kernel image and page-table teardown; the 8042 line achieves the
  same result with three instructions.
