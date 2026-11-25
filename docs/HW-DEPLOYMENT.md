# Hardware deployment roadmap — flashing an empty LAN server

- **Status**: H1 recon complete (2026-11-22) — access decision pending
- **Date**: 2026-11-22
- **Goal**: Phase 2 exit criterion "microcore boots QEMU -> hardware"
  (docs/goals/PHASE-STATUS.md), and the K2 real-silicon claim for the
  NVMe driver.

## H1 recon results (192.168.1.191)

| Field | Value |
|---|---|
| Machine | Lenovo, x86_64, CachyOS (kernel 7.2.4), 8 GiB RAM |
| NVMe | WDC PC SN720 256G (`nvme0n1`) — real silicon, present |
| Services | SSH (OpenSSH 10.6, keys not yet authorized), Go exporter :9100, HTTP :8000 |
| Firewall | everything else filtered |
| Serial | **none** — modern Lenovo: no 16550 COM1; omniosk's only output today is COM1 |
| BMC/IPMI | not detected (consumer/prosumer Lenovo: no IPMI) |

## The pivot this machine forces

**Serial is dead on this class of hardware.** The kernel's only
output is COM1 PortIo; on the target there is no UART. Two additions
make the smoke observable without serial:

1. **VGA text console** (`0xB8000` + CRTC cursor) — GRUB leaves the
   text framebuffer active at hand-off; the kernel mirrors its output
   there. Simple, no USB stack needed.
2. **Evidence to disk** — the smoke writes its marker report to the
   NVMe itself (which also proves the driver end-to-end), and after a
   reboot back into CachyOS the report is read from disk. **The disk
   is the serial console.**

Shutdown path: `kexec` back into CachyOS (if SSH is granted), the
legacy keyboard-controller reboot port, or a manual power cycle.

## Access decision (the user's call)

- **Option S — SSH key** (recommended): append the estate pubkey to
  the server's `authorized_keys`; then the whole flow is scriptable —
  GRUB install to the NVMe (omniosk boots persistently = the literal
  "flash"), kexec round-trips, report retrieval.
- **Option U — USB stick**: the user plugs a stick and picks it in the
  boot menu per test; no credentials needed, but every boot needs a
  physical hand.
- Both work; S is one `authorized_keys` line away.

## The four QEMU couplings (audited 2026-11-22)

| # | Coupling | Location | Fix |
|---|---|---|---|
| 1 | PVH ELF boot (QEMU scans ELF notes; ESI -> hvm_start_info) | main.rs boot asm | multiboot2 header + info parsing, entry branches on protocol |
| 2 | PCI scan matches QEMU's vendor/device (0x1B36:0x0010), bus 0, CAM | nvme.rs PCI scan | class-code 0x0108 scan (WDC = vendor 0x15B7); multi-bus/ECAM if needed |
| 3 | isa-debug-exit ends the smoke | omniosk::qemu_exit | boot-profile flag; hardware gate = VGA output + NVMe report + timeout |
| 4 | Memory map via PVH pointer; fixed heap address | main.rs memmap | multiboot2 map parse; heap placed from the map (ACPI/EBDA collision check) |

Low-risk keepers: COM1 PortIo writes are harmless on port-less
machines; PIT/PIC timing; polling NVMe (no interrupt plumbing);
identity 1 GiB page tables.

## Roadmap

- **H1 recon** — DONE (2026-11-22, above).
- **H2 boot-chain parity** (QEMU-only, safe): multiboot2 entry, GRUB
  ISO build script, class-code NVMe scan, VGA text console, exit-path
  flag. The gate: QEMU boots the ISO through GRUB with all markers
  green on the VGA console.
- **H3 deploy + boot**: SSH path — GRUB install to the NVMe + kexec
  round-trips; USB path — stick + boot-menu pick. Serial replaced by
  VGA + the NVMe report.
- **H4 real NVMe**: the driver against the SN720; the smoke markers in
  the on-disk report close the K2 claim.
- **H5 gate redefined**: `scripts/hw-omniosk.sh` — boot, wait, reboot
  back, read the report from disk; `make hw-smoke`; PHASE-STATUS
  Phase 2 hardware criterion closes.

## Rules

- Never debug boot blockers blind: H2's QEMU-GRUB parity exists so
  every hardware difference is isolated from boot-chain differences.
- The server is boot-to-RAM: nothing is installed unless Option S is
  chosen AND the user confirms a persistent GRUB install to the NVMe
  (that overwrites the CachyOS bootloader — reversible, but state it).
- Each marker lost between QEMU and hardware is one bounded debug
  session — VGA + disk evidence first, always.
