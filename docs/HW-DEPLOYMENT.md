# Hardware deployment roadmap — flashing an empty LAN server

- **Status**: plan (H1 pending recon; the boot-mechanism ADR follows it)
- **Date**: 2026-11-22
- **Goal**: Phase 2 exit criterion "microcore boots QEMU -> hardware"
  (docs/goals/PHASE-STATUS.md), and the K2 real-silicon claim for the
  NVMe driver.

## The four QEMU couplings (audited 2026-11-22)

| # | Coupling | Location | Fix |
|---|---|---|---|
| 1 | PVH ELF boot (QEMU scans ELF notes; ESI -> hvm_start_info) | main.rs boot asm | multiboot2 header + info parsing, entry branches on protocol |
| 2 | PCI scan matches QEMU's vendor/device (0x1B36:0x0010), bus 0, CAM | nvme.rs PCI scan | class-code 0x0108 scan; multi-bus/ECAM if needed |
| 3 | isa-debug-exit ends the smoke | omniosk::qemu_exit | boot-profile flag; hardware gate = serial markers + timeout |
| 4 | Memory map via PVH pointer; fixed heap address | main.rs memmap | multiboot2 map parse; heap placed from the map (ACPI/EBDA collision check) |

Low-risk keepers: COM1 serial (16550), PIT/PIC timing, polling NVMe
(no interrupt plumbing), identity 1 GiB page tables.

## Roadmap

- **H1 recon** (no flashing): identify the server (BMC type/access,
  CPU/RAM/NVMe inventory, serial route, PXE or virtual media).
  Deliverable: inventory + boot-mechanism pick (multiboot2+GRUB
  recommended; UEFI stub only if GRUB is unavailable).
- **H2 boot-chain parity** (QEMU-only, safe): multiboot2 entry, GRUB
  ISO build script, class-code NVMe scan, exit-path flag. The gate:
  QEMU boots the ISO through GRUB with all markers green.
- **H3 deploy + boot**: deliver the ISO (virtual media > PXE), boot,
  serial via SOL/physical, iterate on blockers (memory collisions,
  firmware hand-off).
- **H4 real NVMe**: the driver against real silicon; the smoke markers
  on a real drive close the K2 claim.
- **H5 gate redefined**: `scripts/hw-omniosk.sh` + `make hw-smoke`
  (serial marker watching with timeout); PHASE-STATUS Phase 2 hardware
  criterion closes; the tick-to-trade plane gets a real board.

## Rules

- Never debug boot blockers blind: H2's QEMU-GRUB parity exists so
  every hardware difference is isolated from boot-chain differences.
- The server is boot-to-RAM: nothing is installed; every boot is from
  the delivered image; reboots are free.
- Each marker lost between QEMU and hardware is one bounded debug
  session — serial evidence first, always.
