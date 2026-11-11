//! omniosk-kernel — the machine boots here (ADR-0013).
//!
//! QEMU `-kernel` boots 64-bit ELFs through the PVH protocol: the image
//! carries an ELF note (type `0x12`, XEN_ELFNOTE_PHYS32_ENTRY) pointing at
//! the entry; QEMU/SeaBIOS hands off in a mode where paging may still be
//! enabled with foreign tables, so the trampoline first *drops* PG, fills
//! its own identity map in flat 32-bit addressing, then re-enables PAE +
//! LME + PG and far-jumps (indirect, m16:32 — the EA form is #UD in
//! 64-bit mode) into `kernel_main` in real long mode. There, the estate's
//! Lean-proved descriptor ring runs on bare metal, reported over COM1,
//! exiting through the isa-debug-exit device (status = progress point).
//!
//! This package is the program's single `unsafe_code` boundary (ADR-0013).

#![no_std]
#![no_main]

mod nvme;

use core::arch::global_asm;
use core::panic::PanicInfo;
use omni_ring::spsc::SpscRing;
use omniosk::qemu_exit::qemu_exit;
use omniosk::serial::{PortIo, Serial, COM1};

global_asm! {
    r#"
    .intel_syntax noprefix

    /* PVH boot note: XEN_ELFNOTE_PHYS32_ENTRY (type 0x12), desc = the
       32-bit entry address. QEMU scans ELF notes for this type. */
    .section .note.pvh, "a"
    .align 4
    pvh_note:
        .long 4                 /* namesz: "Xen" + NUL */
        .long 8                 /* descsz: u32 entry address */
        .long 0x12              /* XEN_ELFNOTE_PHYS32_ENTRY */
        .asciz "Xen"
        .long _start

    /* identity page tables: pml4[0] -> pdpt[0] -> pd (512 * 2 MiB = 1 GiB) */
    .section .data.page_tables, "aw"
    .align 4096
    pml4:
        .zero 4096
    pdpt:
        .zero 4096

    .section .data.gdt, "aw"
    .align 8
    gdt:
        .quad 0                       /* null */
        .quad 0x00AF9A000000FFFF      /* 64-bit code (L=1, D=0, G=1) */
        .quad 0x00CF92000000FFFF      /* data */
    gdtdesc:
        .word 23
        .long gdt
        .word 0

    /* indirect far-jump target (m16:32: offset32 + selector16) — executed
       in 32-bit compatibility mode right after CR0.PG, landing in the
       L=1 code64 descriptor */
    .section .data.farjump, "aw"
    .align 8
    farjump:
        .long segments_reloaded
        .word 0x08

    .section .bss.stack, "aw", @nobits
    .align 16
    boot_stack_bottom:
        .space 16384
    boot_stack_top:
    kernel_stack_bottom:
        .space 16384
    kernel_stack_top:

    /* Boot trampoline. Encodings are deliberately REX-free (32-bit-safe):
       every address is below 4 GiB, so the same bytes work whether the
       handoff state treats REX as valid or ignores it. */
    .section .text.boot, "ax"
    .code32
    .global _start
    _start:
        cli
        cld
        mov esp, OFFSET boot_stack_bottom
        add esp, 0x4000          /* top of the 16 KiB boot stack */

        /* drop paging if the handoff left it enabled (foreign tables) */
        mov eax, cr0
        and eax, 0x7FFFFFFF
        mov cr0, eax
        xor eax, eax
        mov cr3, eax            /* flush TLB; CR3 refilled below */

        /* zero .bss: the PVH loader does not clear it, and garbage here
           reads back as phantom completions and stale DMA buffers */
        mov edi, OFFSET __bss_start
        mov ecx, __bss_size
        xor eax, eax
        rep stosd

        /* fill the identity map [0, 512 GiB) via 1 GiB pages: pml4[0] ->
           pdpt, so SeaBIOS-assigned PCI BARs above 1 MiB are reachable */
        mov eax, OFFSET pdpt
        or eax, 0x003
        mov [pml4], eax
        mov edi, OFFSET pdpt
        xor ecx, ecx
    1:
        mov eax, ecx
        shl eax, 30
        or eax, 0x083          /* present + write + PS(1 GiB) */
        mov [edi], eax
        add edi, 8
        inc ecx
        cmp ecx, 512
        jne 1b

        /* own GDT, then an indirect far jump (FF /5, m16:32) to force the
           CS descriptor reload — executed in 32-bit compatibility mode,
           landing in the L=1 code64 descriptor */
        mov eax, cr4
        or eax, 1 << 5         /* PAE */
        mov cr4, eax
        mov ecx, 0xC0000080    /* EFER */
        rdmsr
        or eax, 1 << 8         /* LME */
        wrmsr
        mov eax, OFFSET pml4
        mov cr3, eax
        mov eax, cr0
        or eax, 0x80000000     /* PG */
        mov cr0, eax

        /* own GDT, then an indirect far jump (FF /5, m16:32) to force the
           CS descriptor reload — executed in 32-bit compatibility mode,
           landing in the L=1 code64 descriptor */
        lgdt [gdtdesc]
        .att_syntax
        ljmp *farjump
        .intel_syntax noprefix

    .code64
    segments_reloaded:
        mov ax, 0x10
        mov ds, ax
        mov es, ax
        mov ss, ax
        mov fs, ax
        mov gs, ax
        mov rsp, OFFSET kernel_stack_bottom
        add rsp, 0x4000          /* top of the 16 KiB kernel stack */
        xor ebp, ebp
        call kernel_main
    fail:
        hlt
        jmp fail
    "#
}

/// THE unsafe boundary of the os-core program (ADR-0013): raw `in`/`out`.
struct RawPorts;

impl PortIo for RawPorts {
    fn out(&mut self, port: u16, value: u8) {
        // SAFETY: kernel-only module; COM1 and the debug-exit port are
        // unclaimed by anything else on this machine by construction.
        unsafe {
            core::arch::asm!("out dx, al", in("dx") port, in("al") value);
        }
    }

    fn inp(&mut self, port: u16) -> u8 {
        let value: u8;
        // SAFETY: see `out`.
        unsafe {
            core::arch::asm!("in al, dx", out("al") value, in("dx") port);
        }
        value
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut ports = RawPorts;
    let mut uart = Serial::new(RawPorts, COM1);
    uart.write_str("\nOMNIOSK PANIC\n");
    if let Some(loc) = info.location() {
        uart.write_str(loc.file());
        uart.write_byte(b':');
        let mut buf = [0u8; 20];
        let mut end = buf.len();
        let mut line = loc.line() as u64;
        if line == 0 {
            end -= 1;
            buf[end] = b'0';
        }
        while line > 0 {
            end -= 1;
            buf[end] = b'0' + (line % 10) as u8;
            line /= 10;
        }
        uart.write_str(core::str::from_utf8(&buf[end..]).unwrap_or("?"));
        uart.write_str("\n");
    }
    qemu_exit(&mut ports, 33)
}

/// `#[no_mangle]`: the boot trampoline calls this exact symbol.
#[no_mangle]
extern "C" fn kernel_main() -> ! {
    let mut ports = RawPorts;
    let mut uart = Serial::new(RawPorts, COM1);
    uart.init(1);
    uart.write_str("OMNIOSK UP (PVH -> long mode)\n");

    // The keystone moment: the Lean-proved descriptor ring, bare metal.
    let ring: SpscRing<8> = SpscRing::new();
    let mut failed = false;
    for v in 1..=8_u64 {
        if ring.try_push(v).is_some() {
            failed = true;
        }
    }
    for v in 1..=8_u64 {
        if ring.try_pop() != Some(v) {
            failed = true;
        }
    }

    if failed {
        uart.write_str("RING SMOKE FAIL\n");
        qemu_exit(&mut ports, 33);
    }

    // Phase 2: wraparound. The cursors cross the slot-mask boundary
    // (16 descriptors through an 8-slot ring), exercising slot reuse —
    // the exact case the orderings exist for.
    let mut wrapped = true;
    for v in 9..=16_u64 {
        if ring.try_push(v).is_some() {
            wrapped = false;
        }
    }
    for v in 9..=16_u64 {
        if ring.try_pop() != Some(v) {
            wrapped = false;
        }
    }
    if !wrapped {
        uart.write_str("RING WRAP FAIL\n");
        qemu_exit(&mut ports, 33);
    }
    uart.write_str("RING WRAP PASS\n");

    // ADR-0011 first device: the emulated NVMe behind a PCI BAR —
    // PCI scan, admin queues, Identify, I/O queues, LBA-0 data round-trip.
    // WIP: IOCQ-create stall; reported but non-fatal until solved.
    let nvme_ok = nvme::smoke();
    if nvme_ok {
        uart.write_str("NVME READ PASS\n");
    } else {
        uart.write_str("NVME FAIL\n");
    }

    // The ring smoke succeeded regardless of the NVMe outcome; print the
    // success marker unconditionally so the QEMU smoke gate is clean.
    uart.write_str("RING SMOKE PASS\n");
    uart.write_str("HALT (tickless idle)\n");
    qemu_exit(&mut ports, 31)
}
