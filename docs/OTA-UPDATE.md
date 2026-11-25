# OTA update design — the keyboard-less deployment loop

- **Status**: design accepted; implementation pending (the next kernel
  tranche)
- **Date**: 2026-11-25
- **Depends on**: the hardware-mode smoke (landed), the NVMe read path
  (landed, hardware-validated pending screen verdict), the bridge-
  walking PCI scan (landed)

## The constraint

The hardware target (Lenovo, 192.168.1.191) has no serial port, no
BMC, and — per the operator — no keyboard will be attached. Once
omniosk is the Limine default entry, the machine boots omniosk on
every power-cycle and CachyOS never returns on its own. The ESP write
needed to change the kernel is therefore only possible while CachyOS
is up — which, after the first omniosk boot, requires a keyboard.

The design below removes CachyOS from the update path entirely: the
running omniosk stages and loads its own successor, and the operator
PC receives the evidence over UDP.

## The update channel: reserved LBAs 34..165

The GPT-to-first-partition alignment gap on the target disk: the GPT
header + entries occupy LBAs 0..33; the first partition (the ESP)
starts at LBA 2048. LBAs 34..~165 sit in the alignment gap, owned by
nothing, invisible to every filesystem, safe to read and write.

Layout:

| LBA | Content |
|---|---|
| 34 | update header: magic `OMUP` u32, ELF size u32 |
| 35.. | the new `omniosk.elf` file bytes, contiguous |

The publisher (CachyOS, over SSH) writes both with `dd`; the loader
(omniosk, at boot) validates and consumes. Clearing the header magic
acknowledges consumption.

## The loader (kernel, hardware boot path)

After the smoke passes and the CMOS markers are written:

1. Read LBA 34; require the `OMUP` magic and a plausible size
   (64 <= size <= 512 KiB). No magic -> no update; continue to the
   normal halt.
2. Copy the ELF (size bytes, from LBA 35) into the staging region at
   physical 0x2000000 (32 MiB — above the heap, inside the 128 MiB
   QEMU mapping and the 8 GiB real mapping).
3. Validate the ELF64 header (`0x464C457F` magic, 64-bit, x86-64
   machine) and walk the program headers: each PT_LOAD segment is
   copied from the staging image (`p_offset`) to its load address
   (`p_paddr`), zero-filling `p_memsz - p_filesz`.
4. Build the trampoline at physical 0x1800000 (24 MiB): 26 bytes of
   hand-assembled 32-bit code (see below) plus the parameter
   immediates baked in: staging source, load destination, dword count,
   the update magic for EAX, and the new kernel's 32-bit entry.
5. Build a GDT descriptor: the existing GDT gains a 32-bit code
   segment (L=0, D=1, G=1, selector 0x18) via a refreshed `lgdt`
   (limit 47) — the current GDTR limit (23) does not cover it.
6. The far-jump: `ljmp fword [offset = 0x1800000, selector = 0x18]`
   drops 64-bit long mode to 32-bit compatibility mode (paging still
   on, identity-mapped).
7. The trampoline: clears `CR0.PG`, flushes CR3, performs the `rep
   movsd` copy, loads EAX with the update magic (0x55504441), and
   `jmp eax` — the new kernel's 32-bit entry, which branches to
   `boot_kind = 3` (the update-loaded hardware profile: read-only
   smoke, CMOS markers, halt).

## The loop and its breaker

- A successful update boot clears the header magic at LBA 34 and
  writes its own CMOS markers — the next boot sees no staged update
  and runs normally (halting after the smoke).
- A failed update boot (crash before the clear) leaves the staging
  intact: the next boot re-runs the SAME update — deterministic, no
  corruption.
- The CMOS jump counter (offset 0x3E) increments per update jump;
  at 3 the loader refuses further jumps and halts with a distinct
  banner — the loop-breaker. Cleared by a successful full smoke.

## The evidence loop

Once live, the full remote iteration needs no keyboard:

1. The operator builds a new kernel and `dd`s it to LBAs 35+ plus the
   header to LBA 34 over SSH from CachyOS.
2. `ssh sudo systemctl reboot`.
3. Limine -> OMNIOSK -> the smoke -> the update load -> the new kernel
   -> its smoke -> its UDP markers -> halt.
4. The operator reads the UDP log on this PC. Restore to CachyOS
   whenever convenient (one physical menu pick).

The RTL8168 UDP emission (the `rtl8168` module, transmit-only, landed
but unwired) delivers the per-boot markers; a future tranche adds the
receive path so the operator can push updates to a RUNNING omniosk
without the CachyOS `dd` step at all.

## What must be true for the first jump

- The boot-1 screen verdict: the smoke must have reached
  `NVME READ CONSISTENT (hw)` — the read path is the update loader's
  only dependency.
- The staging/trampoline addresses (32 MiB / 24 MiB) must be inside
  real RAM (8 GiB machine ✓, 128 MiB QEMU ✓).
- The GDT extension and the compat-mode drop follow the Intel SDM's
  leaving-long-mode sequence; a failure there is a triple fault and a
  reboot — the CMOS jump counter prevents an infinite cycle.
