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

mod cmos;
mod nvme;
mod rtl8168;
mod vga;

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

    /* Multiboot2 header (GRUB/ISO/Limine boot path): must live within the
       first 32 KiB of the file and be 64-bit aligned. We ask for the
       memory map (MBI tag 6) via the information-request tag, and the
       entry-address tag routes GRUB/Limine to the same 32-bit entry the
       PVH path uses (EAX = 0x36D76289, EBX = &MBI).
       Tag fields: type u16, flags u16, size u32, then the body. Tags
       are 8-aligned by the loader's stride (ALIGN_UP(size, 8)), so each
       tag carries explicit padding to the next 8-boundary — an unpadded
       12-byte tag makes Limine read the NEXT tag from inside the
       padding and panic "Header tag exceeds header bounds". Found by
       the Limine-in-QEMU gate after the hardware deployment panicked
       on the real machine. */
    .section .multiboot, "a"
    .align 8
    mb2_header:
        .long 0xE85250D6                    /* magic */
        .long 0                             /* arch: i386 */
        .long mb2_header_end - mb2_header   /* header length */
        .long -(0xE85250D6 + mb2_header_end - mb2_header) /* checksum */
        /* information request tag: the memory map (required) */
        .short 1                            /* type */
        .short 0                            /* flags */
        .long 12                            /* size (raw, unpadded) */
        .long 6                             /* MBI tag 6: memory map */
        .long 0                             /* pad: stride to the next 8 */
        /* entry address tag */
        .short 3                            /* type */
        .short 0                            /* flags */
        .long 12                            /* size */
        .long _start                        /* entry: the 32-bit trampoline */
        .long 0                             /* pad */
        /* end tag */
        .short 0                            /* type */
        .short 0                            /* flags */
        .long 8                             /* size */
    mb2_header_end:

    /* PVH handoff pointer: ESI at the 32-bit entry = &hvm_start_info
       (Xen PVH abi; QEMU fills it with the e820-style memory map).
       Saved into .data — NOT .bss, which the trampoline zeroes after
       this store would already have been wiped. */
    .section .data.memmap, "aw"
    .align 4
    .global pvh_info_ptr
    pvh_info_ptr:
        .long 0

    /* which boot protocol handed off: 1 = PVH, 2 = multiboot2,
       3 = OTA update (trampoline, EAX = UPDA, no protocol struct) */
    .section .data.bootkind, "aw"
    .align 4
    .global boot_kind
    boot_kind:
        .long 0

    /* identity page tables: pml4[0|1] -> pdpt[0|1] -> 1 GiB pages.
       Two PDPTs (1 TiB of identity map) because OVMF on the pc machine
       parks the NVMe BAR in the 64-bit PCI hole at 0xC0_00000000
       (768 GiB, PML4 index 1) — one PDPT only covers [0, 512 GiB). */
    .section .data.page_tables, "aw"
    .align 4096
    pml4:
        .zero 4096
    pdpt:
        .zero 4096
    pdpt1:
        .zero 4096

    .section .data.gdt, "aw"
    .align 8
    gdt:
        .quad 0                       /* null */
        .quad 0x00AF9A000000FFFF      /* 64-bit code (L=1, D=0, G=1) */
        .quad 0x00CF92000000FFFF      /* data */
        .quad 0x00CF9A000000FFFF      /* 32-bit code (L=0, D=1, G=1) —
                                         selector 0x18, the OTA
                                         trampoline's compat target */
    gdtdesc:
        .word 31
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
        /* boot-protocol branch: MB2 hand-off is EAX=0x36D76289 with
           EBX=&MBI; PVH hand-off is ESI=&hvm_start_info (EAX is
           garbage). Save both the pointer and which protocol won. */
        cmp eax, 0x36D76289
        je .multiboot2
        cmp eax, 0x55504441
        je .ota_update
        mov [pvh_info_ptr], esi         /* PVH: ESI = &hvm_start_info */
        mov eax, 1                      /* 1 = PVH (QEMU -kernel) */
        jmp .set_kind
    .multiboot2:
        mov eax, 2                      /* 2 = multiboot2 (GRUB) */
        jmp .set_kind
    .ota_update:
        mov eax, 3                      /* 3 = OTA update handoff — the
                                           trampoline's UPDA magic; no
                                           protocol struct exists */
    .set_kind:
        mov [boot_kind], eax            /* EAX width sizes the store */
    .entered:
        mov esp, OFFSET boot_stack_bottom
        add esp, 0x4000          /* top of the 16 KiB boot stack */

        /* drop paging if the handoff left it enabled (foreign tables) */
        mov eax, cr0
        and eax, 0x7FFFFFFF
        mov cr0, eax
        xor eax, eax
        mov cr3, eax            /* flush TLB; CR3 refilled below */

        /* zero .bss: the PVH loader does not clear it, and garbage here
           reads back as phantom completions and stale DMA buffers.
           OFFSET is required: a bare symbol assembles as a memory LOAD
           (relocated to the symbol's VALUE as the address — the count
           then comes from video-RAM garbage at 0xa154), not an
           immediate. Found by the OTA gate's boot-2 regression. */
        mov edi, OFFSET __bss_start
        mov ecx, OFFSET __bss_size
        xor eax, eax
        rep stosd

        /* fill the identity map [0, 1 TiB) via 1 GiB pages: pml4[0] ->
           pdpt, pml4[1] -> pdpt1, so SeaBIOS-assigned PCI BARs above
           1 MiB AND OVMF's 64-bit hole (768 GiB) are reachable */
        mov eax, OFFSET pdpt
        or eax, 0x003
        mov [pml4], eax
        mov eax, OFFSET pdpt1
        or eax, 0x003
        mov [pml4 + 8], eax
        mov edi, OFFSET pdpt
        xor ecx, ecx
    1:
        /* PDPE for a 1 GiB page at ecx * 2^30. Write BOTH dwords: the
           high dword carries base bits 63:32, so without it every entry
           >= 4 GiB silently maps VA back to physical 0. */
        mov eax, ecx
        shl eax, 30
        or eax, 0x083          /* present + write + PS(1 GiB) */
        mov [edi], eax
        mov eax, ecx
        shr eax, 2             /* base bits 63:32 = ecx >> 2 */
        mov [edi + 4], eax
        add edi, 8
        inc ecx
        cmp ecx, 512
        jne 1b
        mov edi, OFFSET pdpt1
    2:
        /* ecx continues at 512: pdpt1 covers VA [512 GiB, 1 TiB). */
        mov eax, ecx
        shl eax, 30
        or eax, 0x083
        mov [edi], eax
        mov eax, ecx
        shr eax, 2
        mov [edi + 4], eax
        add edi, 8
        inc ecx
        cmp ecx, 1024
        jne 2b

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
    .section .text.int, "ax"
    .global timer_stub
    timer_stub:
        push rax
        push rcx
        push rdx
        push rbx
        push rbp
        push rsi
        push rdi
        push r8
        push r9
        push r10
        push r11
        push r12
        push r13
        push r14
        push r15
        call timer_tick
        pop r15
        pop r14
        pop r13
        pop r12
        pop r11
        pop r10
        pop r9
        pop r8
        pop rdi
        pop rsi
        pop rbp
        pop rbx
        pop rdx
        pop rcx
        pop rax
        iretq

    /* Fault gates: loud failures. The faulting context is dead, so no
       register preservation — the handler prints the vector and halts.
       CPU-pushed error codes are popped into EDI (discarded) where the
       vector has one; the vector number then rides in EDI (SysV arg 0). */
    .section .text.int, "ax"
    .code64
    .global fault_stub_ud
    fault_stub_ud:                 /* vector 6: no error code */
        mov edi, 6
        jmp fault_common
    .global fault_stub_df
    fault_stub_df:                 /* vector 8: DF pushes a zero code */
        pop rdi
        mov edi, 8
        jmp fault_common
    .global fault_stub_gp
    fault_stub_gp:                 /* vector 13: error code */
        pop rdi
        mov edi, 13
        jmp fault_common
    .global fault_stub_pf
    fault_stub_pf:                 /* vector 14: error code */
        pop rdi
        mov edi, 14
        jmp fault_common
    fault_common:
        call fault_tick
    1:
        hlt
        jmp 1b

    /* Spurious interrupts: no EOI, no handler work — just return.
       MUST NOT share the timer's vector: a spurious looped through the
       timer handler starves the main loop (fetch_add grows forever). */
    .global spurious_stub
    spurious_stub:
        lock inc DWORD PTR [rip + spurious_count]
        iretq

    .section .data.spurious, "aw"
    .global spurious_count
    spurious_count:
        .long 0

    fail:
        hlt
        jmp fail
    "#
}

// ---- PVH memory map + heap (REQ-OS-005 seam, REQ-OS-007 consumer) ----

use omni_heap::{PageAlloc, PAGE_SIZE};

extern "C" {
    /// Written by the trampoline: ESI at the PVH 32-bit entry, i.e. the
    /// physical address of the `hvm_start_info` struct.
    static pvh_info_ptr: u32;
    /// Written by the trampoline at the 32-bit entry: which boot
    /// protocol handed off. 1 = PVH (QEMU `-kernel`, exits via
    /// isa-debug-exit), 2 = multiboot2 (GRUB/ISO, halts — the smoke
    /// gate watches the serial log instead of the exit code).
    static boot_kind: u32;
    /// Linker-script symbol (kern.ld): end of the kernel image.
    static __bss_end: u8;
}

/// Which boot protocol brought us up (read once at boot).
fn boot_protocol() -> u32 {
    // SAFETY: written by the trampoline before any Rust runs; never
    // mutated afterwards.
    unsafe { core::ptr::addr_of!(boot_kind).read_volatile() }
}

/// Bitmap backing for the microcore heap: 512 words -> 32,768 pages
/// (128 MiB) maximum. Lives in .bss, zeroed by the trampoline.
const HEAP_BITMAP_WORDS: usize = 512;
const HEAP_MAX_PAGES: usize = HEAP_BITMAP_WORDS * 64;
static mut HEAP_BITMAP: [u64; HEAP_BITMAP_WORDS] = [0; HEAP_BITMAP_WORDS];

const PVH_MAGIC: u32 = 0x336ec578;

/// Locates `hvm_start_info`: trusts the trampoline-latched ESI when it
/// points at a valid struct, else scans conventional low memory for the
/// magic. DISCOVERY (2026-11-13): with QEMU 11.1.3's SeaBIOS, ESI at the
/// PVH entry parks at 0x560 but the real struct is a SeaBIOS low-malloc
/// elsewhere (found at 0x1580) — the spec pointer cannot be trusted.
fn pvh_info_base() -> Option<usize> {
    let ptr = unsafe { core::ptr::addr_of!(pvh_info_ptr).read_volatile() } as usize;
    let magic_le = PVH_MAGIC.to_le_bytes();
    let magic_at = |off: usize| -> bool {
        if off < 4 {
            return false;
        }
        for (k, eb) in magic_le.iter().enumerate() {
            // SAFETY: fixed-offset byte reads inside mapped low RAM /
            // the SeaBIOS f-segment; same boundary as nvme.rs MMIO reads.
            let vb = unsafe { ((off + k) as *const u8).read_volatile() };
            if vb != *eb {
                return false;
            }
        }
        true
    };
    if ptr != 0 && magic_at(ptr) {
        return Some(ptr);
    }
    let ranges = [(0x400usize, 0x9FC00usize), (0xF0000usize, 0x100000usize)];
    for (lo, hi) in ranges {
        for off in (lo..hi).step_by(4) {
            if magic_at(off) {
                return Some(off);
            }
        }
    }
    None
}

/// Reads `hvm_start_info` (Xen PVH abi, filled by SeaBIOS): returns
/// `(memmap_paddr, memmap_entries)` or None if the struct is absent.
fn pvh_memmap() -> Option<(usize, u32)> {
    let base = pvh_info_base()?;
    let p = base as *const u32;
    // SAFETY: hvm_start_info is 32 bytes of plain u32/u64 fields; the
    // reads use the documented offsets (magic 0, memmap_entries 28,
    // memmap_paddr 32). read_volatile keeps the compiler from inventing
    // wider loads than the field reads.
    unsafe {
        let entries = p.add(7).read_volatile();
        let map_paddr = p.add(8).read_volatile();
        Some((map_paddr as usize, entries))
    }
}

/// Picks the largest RAM (e820 type 1) region that starts above the
/// kernel image, returning `(heap_base, pages)` capped to the bitmap.
fn best_ram_above(min_addr: usize) -> Option<(usize, usize)> {
    let (map_paddr, entries) = pvh_memmap()?;
    let mut best: Option<(usize, usize)> = None;
    for i in 0..entries as usize {
        let e = map_paddr + i * 24; // hvm_memmap_table_entry is 24 bytes
        // SAFETY: fixed-offset field reads inside the memmap table;
        // see the SAFETY note in pvh_memmap.
        let (rtype, paddr, length) = unsafe {
            let t = (e as *const u32).read_volatile();
            let p = ((e + 8) as *const u64).read_volatile() as usize;
            let l = ((e + 16) as *const u64).read_volatile() as usize;
            (t, p, l)
        };
        if rtype != 1 {
            continue; // e820 RAM only
        }
        let start = paddr.max(min_addr).div_ceil(PAGE_SIZE) * PAGE_SIZE;
        let end = paddr.saturating_add(length);
        if start >= end {
            continue;
        }
        let pages = (end - start) / PAGE_SIZE;
        if pages == 0 {
            continue;
        }
        if best.map(|(_, p)| pages > p).unwrap_or(true) {
            best = Some((start, pages));
        }
    }
    best
}

/// Carves the heap out of the PVH memory map and smoke-tests it:
/// allocate two pages, free the first, and require first-fit to hand
/// it straight back. Any violation prints HEAP FAIL (and the marker
/// gate in scripts/qemu-omniosk.sh turns red).
fn heap_init(uart: &mut Serial<RawPorts>) -> Option<PageAlloc<'static>> {
    use omni_heap::PAGE_SIZE;

    // &__bss_end as an address: the linker symbol, not the pointee.
    let bss_end = core::ptr::addr_of!(__bss_end) as usize;
    let Some((base, pages)) = best_ram_above(bss_end) else {
        // Firmware (this SeaBIOS build) leaves hvm_start_info.memmap
        // empty. Fall back to a conservative 16 MiB heap directly above
        // the kernel image — safe for any QEMU -m >= 32M, and the
        // firmware can never hand that range to anyone else: it sits
        // inside the region QEMU reports as RAM and below the ACPI
        // reservation at the top of low memory.
        let start = bss_end.div_ceil(PAGE_SIZE) * PAGE_SIZE;
        uart.write_str("heap: memmap empty, conservative 16 MiB @ ");
        write_hex32(uart, start as u32);
        uart.write_str("\n");
        return heap_smoke(uart, start, (16 << 20) / PAGE_SIZE);
    };
    heap_smoke(uart, base, pages.min(HEAP_MAX_PAGES))
}

/// Builds the heap over `(base, pages)`, smoke-tests first-fit
/// determinism (alloc two, free the first, realloc must return it), and
/// returns the live allocator for downstream consumers (NVMe DMA pages).
fn heap_smoke(
    uart: &mut Serial<RawPorts>,
    base: usize,
    pages: usize,
) -> Option<PageAlloc<'static>> {
    let pages = pages.min(HEAP_MAX_PAGES);

    // SAFETY: single-threaded boot context; HEAP_BITMAP is the sole
    // owner of the bitmap words (same static-mut pattern as nvme.rs).
    // The slice reassembly cannot produce None (fixed non-zero length);
    // on the impossible path the boot halts (no_std: no std::abort).
    let bitmap: &'static mut [u64] = match unsafe {
        (core::ptr::slice_from_raw_parts_mut(
            core::ptr::addr_of_mut!(HEAP_BITMAP).cast::<u64>(),
            HEAP_BITMAP_WORDS,
        ))
        .as_mut()
    } {
        Some(s) => s,
        // The slice reassembly cannot produce None (fixed non-zero
        // length); on the impossible path the boot halts via the
        // kernel's panic handler (panic = abort).
        None => panic!("bitmap reassembly is infallible"),
    };
    let Ok(mut heap) = PageAlloc::new(base, pages, bitmap) else {
        uart.write_str("heap: layout rejected\n");
        return None;
    };

    let Ok(a) = heap.alloc() else {
        uart.write_str("heap: alloc 1 failed\n");
        return None;
    };
    let Ok(b) = heap.alloc() else {
        uart.write_str("heap: alloc 2 failed\n");
        return None;
    };
    if a % PAGE_SIZE != 0
        || b != a + PAGE_SIZE
        || a < base
        || a >= base + pages * PAGE_SIZE
    {
        uart.write_str("heap: layout invariant violated\n");
        return None;
    }
    if heap.free(a) != Ok(()) {
        uart.write_str("heap: free 1 failed\n");
        return None;
    }
    // First-fit determinism on real hardware: the freed page must come
    // straight back.
    if heap.alloc() != Ok(a) {
        uart.write_str("heap: realloc != first-fit\n");
        return None;
    }
    let _ = b;
    Some(heap)
}

/// Writes a 32-bit value as 8 hex nibbles over the serial port.
fn write_hex32(uart: &mut Serial<RawPorts>, v: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for shift in (0..32).step_by(4).rev() {
        let nib = ((v >> shift) & 0xF) as usize;
        let byte = [HEX[nib]];
        uart.write_str(core::str::from_utf8(&byte).unwrap_or("?"));
    }
}

// ---- PIT timer + remapped PIC + omni-task dispatch (REQ-OS-008:
// the microcore gets a real tick and round-robin task slots on it) ----
//
// DISCOVERY (2026-11-14): the LAPIC timer is unusable as the first tick
// source under TCG — QEMU derives the LAPIC clock from the emulated
// TSC, so periods stretch to seconds and periodic re-arm-after-EOI
// never lands inside a smoke window (1 tick per budget). The 8259 PIT
// is emulated against QEMU_CLOCK_VIRTUAL at its real 1.193182 MHz rate:
// deterministic, ms-scale ticks everywhere (TCG, KVM, hardware).
//
// Vector layout: the PIC is remapped to 0x50-0x5F (away from the
// 0x00-0x1F exception range), IRQ0 (PIT ch 0) lands on 0x50.

#[allow(dead_code)] // the extern decls are consumed only via `as usize`
use omni_task::{Scheduler, TaskCtx, TaskFlow};

extern "C" {
    fn timer_stub();
    fn spurious_stub();
    fn fault_stub_ud();
    fn fault_stub_df();
    fn fault_stub_gp();
    fn fault_stub_pf();
}

/// Ticks delivered by the PIT since its last enable.
static TIMER_TICKS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// The CPU reads this table; Rust only writes it through raw casts.
#[repr(align(16))]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct IdtTable([u64; 512]);

static mut IDT: IdtTable = IdtTable([0; 512]);

#[repr(C, packed)]
struct LidtOperand {
    limit: u16,
    base: u64,
}

static mut IDT_DESC: LidtOperand = LidtOperand {
    limit: 4095,
    base: 0,
};

fn outb(port: u16, v: u8) {
    // SAFETY: port I/O is the documented kernel boundary (ADR-0013).
    unsafe { core::arch::asm!("out dx, al", in("dx") port, in("al") v) }
}

/// `rdmsr` — unused now that the tick source is the PIT; kept for the
/// LAPIC calibration follow-up (REQ-OS-008).
#[allow(dead_code)]
fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: privileged instruction in the kernel's unsafe boundary.
    unsafe { core::arch::asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi) };
    ((hi as u64) << 32) | lo as u64
}

/// # Safety
///
/// Called only from `timer_stub` (the IDT gate for vector 0x50), with
/// GP registers preserved by the stub and one PIT interrupt
/// outstanding.
#[no_mangle]
extern "C" fn timer_tick() {
    TIMER_TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    sched_isr_tick();
    // PIC non-specific EOI (OCW2).
    outb(0x20, 0x20);
}

/// Fault gate: prints the vector, halts forever. The smoke timeout reads
/// it as a hard failure — loud, by design.
#[no_mangle]
extern "C" fn fault_tick(vector: usize) -> ! {
    let mut uart = Serial::new(RawPorts, COM1);
    uart.write_str("\nFAULT vec=");
    write_hex32(&mut uart, vector as u32);
    uart.write_str("\n");
    loop {
        core::hint::spin_loop();
    }
}

/// Installs the IDT gate for `vector` -> `handler`.
fn idt_set(vector: u8, handler: usize) {
    let handler = handler as u64;
    // Type 0xF = 64-bit interrupt gate (0xE is the 32-bit one — using
    // it in long mode #GPs on gate delivery, error code = IDT|index).
    //
    // The 64-bit offset is split THREE ways: bits 0-15 at entry bits
    // 0-15, bits 16-31 at entry bits 48-63, bits 32-63 in the high
    // word. Dropping the middle field truncates the handler to its low
    // 16 bits — timer_stub at 0x1002b7 delivered as 0x2b7, landing in
    // SeaBIOS's low-memory code.
    let low = (handler & 0xFFFF)
        | (0x08u64 << 16)          // CS selector
        | (0x8Fu64 << 40)          // present | interrupt gate 64-bit, DPL 0
        | (((handler >> 16) & 0xFFFF) << 48); // offset bits 16-31
    let high = handler >> 32;
    let slot = vector as usize * 2;
    // SAFETY: IDT is a static owned by the boot thread before `sti`.
    unsafe {
        let table = core::ptr::addr_of_mut!(IDT).cast::<u64>();
        core::ptr::write_volatile(table.add(slot), low);
        core::ptr::write_volatile(table.add(slot + 1), high);
    }
}

/// Remaps the 8259 PIC to vectors 0x50-0x5F, unmasks IRQ0 only, and
/// programs PIT channel 0 as a ~1 kHz rate generator.
fn pic_pit_start() {
    // PIC remap, ICW1-4: cascade, vector base 0x50 / 0x58, 8086 mode.
    outb(0x20, 0x11); // master ICW1: init + ICW4
    outb(0xA0, 0x11); // slave ICW1
    outb(0x21, 0x50); // master ICW2: IRQ0-7 -> 0x50-0x57
    outb(0xA1, 0x58); // slave ICW2: IRQ8-15 -> 0x58-0x5F
    outb(0x21, 0x04); // master ICW3: slave on IRQ2
    outb(0xA1, 0x02); // slave ICW3: cascade identity
    outb(0x21, 0x01); // ICW4: 8086 mode
    outb(0xA1, 0x01);
    // Mask everything except IRQ0 (the PIT).
    outb(0x21, 0xFE);
    outb(0xA1, 0xFF);

    // PIT channel 0: mode 2 (rate generator), low/high byte, divisor
    // 1193 -> 1193182 Hz / 1193 = ~1000 Hz.
    outb(0x43, 0x36);
    let divisor = 1193u32;
    outb(0x40, (divisor & 0xFF) as u8);
    outb(0x40, ((divisor >> 8) & 0xFF) as u8);
}

/// LAPIC-timer smoke: interrupts on, spin until the tick counter has
/// advanced `min_ticks` (or the iteration budget is spent). Returns
/// the tick count — PASS requires >= min_ticks.
/// Halts until `ms` milliseconds of PIT ticks have elapsed. With
/// interrupts enabled, `hlt` sleeps the core and wakes on each tick
/// (~1 kHz) — the CPU rests between ticks instead of burning the
/// window. Returns the ticks actually elapsed (the caller compares
/// against `ms` to validate the ~1 kHz calibration).
/// Emits `tag` to the operator PC (192.168.1.1:9933) three times with
/// gaps. Checkpoint telemetry for real hardware: the operator observes
/// boot progress even when a later smoke stage fails before the final
/// report (previously the datagram was emitted only inside the NVMe
/// success path — a failed smoke left the box dark with zero packets).
fn beacon(nic: &mut rtl8168::Nic, uart: &mut Serial<RawPorts>, tag: &str) {
    for _ in 0..3 {
        let _ = nic.send_udp([192, 168, 1, 1], 9933, 40969, tag.as_bytes());
        kernel_sleep_ticks(uart, 300);
    }
}

fn kernel_sleep_ticks(uart: &mut Serial<RawPorts>, target: usize) -> usize {
    use core::sync::atomic::Ordering;
    let start = TIMER_TICKS.load(Ordering::Relaxed);
    let goal = start.saturating_add(target);
    let mut last = start;
    while TIMER_TICKS.load(Ordering::Relaxed) < goal {
        // SAFETY: interrupts are enabled; the PIT wakes the core.
        unsafe { core::arch::asm!("hlt") };
        let now = TIMER_TICKS.load(Ordering::Relaxed);
        if now == last {
            // No tick in a whole halt cycle: the tick source died
            // (masked/unprogrammed) — the sleep would never end.
            uart.write_str("sleep: no tick progress\n");
            return now.saturating_sub(start);
        }
        last = now;
    }
    TIMER_TICKS.load(Ordering::Relaxed).saturating_sub(start)
}

fn timer_smoke(min_ticks: usize, max_iterations: usize) -> usize {
    use core::sync::atomic::Ordering;
    let mut dbg = Serial::new(RawPorts, COM1);
    let mut iters = 0usize;
    while iters < max_iterations {
        let now = TIMER_TICKS.load(Ordering::Relaxed);
        if now >= min_ticks {
            dbg.write_str("[smoke-done tk=");
            write_hex32(&mut dbg, now as u32);
            dbg.write_str("]");
            return now;
        }
        if iters % 5_000_000 == 0 && iters > 0 {
            dbg.write_str("[it=");
            write_hex32(&mut dbg, iters as u32);
            dbg.write_str(" tk=");
            write_hex32(&mut dbg, now as u32);
            dbg.write_str("]");
        }
        iters += 1;
    }
    let now = TIMER_TICKS.load(Ordering::Relaxed);
    dbg.write_str("[budget tk=");
    write_hex32(&mut dbg, now as u32);
    dbg.write_str("]");
    now
}

/// Task slots dispatched by the PIT ISR (round-robin, omni-task).
/// `None` until kernel_main builds the scheduler; the ISR skips
/// dispatch while it is absent (pre-registration ticks are safe).
static mut SCHED: Option<Scheduler<4>> = None;

/// The live NVMe session, handed to the blockcheck task by kernel_main
/// after the smoke (the omni-block seam, task-consumed).
static mut NVME_SESSION: Option<nvme::NvmeSession> = None;

/// 1 = booted on real hardware (multiboot2/GRUB): the NVMe smoke and
/// the blockcheck task validate by read-consistency, not by the magic
/// pattern (the disk carries a live GPT/filesystem). Set once by
/// kernel_main before any task dispatch.
static BOOT_HW: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Blockcheck witnesses: 3 successful LBA-0 reads through the
/// `BlockDevice` trait with zero failures = TASK PASS contribution.
static BLOCKCHECK_OK: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
static BLOCKCHECK_FAIL: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Task: read LBA 0 through the `BlockDevice` trait and verify the
/// magic. Retires after 3 successful reads; a failure retires the
/// task too and latches the fail kind (TASK FAIL).
fn task_blockcheck(ctx: &mut TaskCtx) -> TaskFlow {
    use core::sync::atomic::Ordering;
    use omni_block::BlockDevice;
    let mut dbg = Serial::new(RawPorts, COM1);
    // SAFETY: dispatch context (IF cleared, no reentrancy); the static
    // is the sole owner of the session (same pattern as SCHED).
    let session = unsafe { core::ptr::addr_of_mut!(NVME_SESSION) };
    let present = unsafe { (*session).as_mut().is_some() };
    let session_addr = unsafe { core::ptr::addr_of!(NVME_SESSION) } as usize;
    let session_some = unsafe { (*session).as_ref().is_some() };
    let inner_addr = unsafe {
        core::ptr::addr_of!(NVME_SESSION)
            .as_ref()
            .map_or(0, |opt| opt.as_ref().map_or(0, |s| s as *const _ as usize))
    };
    let lbas_now = unsafe {
        core::ptr::addr_of!(NVME_SESSION)
            .as_ref()
            .map_or(0u64, |opt| opt.as_ref().map_or(0, |s| s.lbas))
    };
    dbg.write_str("blk: present=");
    dbg.write_byte(b'0' + present as u8);
    dbg.write_str(" static_addr=");
    write_hex32(&mut dbg, session_addr as u32);
    dbg.write_str(" some=");
    dbg.write_byte(b'0' + session_some as u8);
    dbg.write_str(" inner=");
    write_hex32(&mut dbg, inner_addr as u32);
    dbg.write_str(" lbas_now=");
    write_hex32(&mut dbg, lbas_now as u32);
    let cur_now = unsafe {
        core::ptr::addr_of!(NVME_SESSION).as_ref().map_or((0, 0, 0), |opt| {
            opt.as_ref().map_or((0, 0, 0), |s| {
                (s.cqe_idx, s.cqe_phase, s.iosq_tail)
            })
        })
    };
    dbg.write_str(" idx=");
    write_hex32(&mut dbg, cur_now.0 as u32);
    dbg.write_str(" ph=");
    write_hex32(&mut dbg, cur_now.1 as u32);
    dbg.write_str(" tail=");
    write_hex32(&mut dbg, cur_now.2 as u32);
    dbg.write_str("\n");
    let Some(s) = (unsafe { (*session).as_mut() }) else {
        BLOCKCHECK_FAIL.fetch_add(1, Ordering::Relaxed);
        return TaskFlow::Done;
    };
    let mut buf = [0u8; 512];
    let mut buf2 = [0u8; 512];
    let read = s.read_lba(0, &mut buf);
    let hw = BOOT_HW.load(core::sync::atomic::Ordering::Relaxed) == 1;
    let ok = if hw {
        let second = s.read_lba(0, &mut buf2);
        read.is_ok()
            && second.is_ok()
            && buf[..32] == buf2[..32]
            && buf.iter().any(|b| *b != 0)
    } else {
        read.is_ok() && buf[..nvme::MAGIC.len()] == *nvme::MAGIC
    };
    if !ok {
        // Fail-path probe: the raw CQE dword for the slot the session
        // poll was waiting on. Phase bit = DW3 bit 16, SF = DW3[30:17],
        // CID = DW3[15:0]. Distinguishes "completion never landed"
        // (dw3 stale), "phase convention mismatch" (P=0 here), and
        // "completed with status error" (SF != 0, P=1).
        let dw3 = nvme::mmio32(s.iocq, 3 * 16 + 12);
        dbg.write_str("bc: raw dw3=");
        write_hex32(&mut dbg, dw3);
        dbg.write_str(" p=");
        dbg.write_byte(b'0' + omni_block::wire::cqe_phase_of(dw3) as u8);
        dbg.write_str(" sf=");
        write_hex32(&mut dbg, (dw3 >> 17) & 0x3FFF);
        dbg.write_str(" cid=");
        write_hex32(&mut dbg, omni_block::wire::cqe_cid(dw3) as u32);
        dbg.write_str(" post idx=");
        write_hex32(&mut dbg, s.cqe_idx as u32);
        dbg.write_str(" ph=");
        write_hex32(&mut dbg, s.cqe_phase as u32);
        dbg.write_str(" tail=");
        write_hex32(&mut dbg, s.iosq_tail as u32);
        dbg.write_str("\n");
        BLOCKCHECK_FAIL.fetch_add(1, Ordering::Relaxed);
        return TaskFlow::Done;
    }
    let _ = ctx;
    let n = BLOCKCHECK_OK.fetch_add(1, Ordering::Relaxed) + 1;
    if n >= 3 {
        TaskFlow::Done
    } else {
        TaskFlow::Continue
    }
}

/// Dispatch bookkeeping for the smoke: a never-done task proves the
/// rotation, a retire-after-3 task proves slot retirement.
static TASK_RUNS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
static RETIRE_RUNS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Demo task A: never retires (the rotation witness).
fn task_blink(_ctx: &mut TaskCtx) -> TaskFlow {
    TASK_RUNS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    TaskFlow::Continue
}

/// Demo task C: sleeps 5 ticks after every dispatch (the Sleep-state
/// witness). First/last dispatch ticks land in statics for the smoke's
/// spacing assertion.
static SLEEPER_RUNS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
static SLEEPER_FIRST: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
static SLEEPER_LAST: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

fn task_sleeper(ctx: &mut TaskCtx) -> TaskFlow {
    use core::sync::atomic::Ordering;
    let t = ctx.ticks as usize;
    if SLEEPER_FIRST.load(Ordering::Relaxed) == 0 {
        SLEEPER_FIRST.store(t, Ordering::Relaxed);
    } else {
        SLEEPER_LAST.store(t, Ordering::Relaxed);
    }
    let _ = SLEEPER_RUNS.fetch_add(1, Ordering::Relaxed);
    TaskFlow::Sleep(5)
}

/// Demo task B: retires after 3 dispatches (the retirement witness).
fn task_retire(_ctx: &mut TaskCtx) -> TaskFlow {
    let n = RETIRE_RUNS.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
    if n >= 3 {
        TaskFlow::Done
    } else {
        TaskFlow::Continue
    }
}

/// The PIT ISR's dispatch step. SAFETY: called only from the timer ISR
/// (IF is cleared inside the handler, so no reentrancy); `SCHED` is
/// initialized once by kernel_main before `sti` and never moved.
fn sched_isr_tick() {
    // SAFETY: see fn docs.
    let sched = unsafe { core::ptr::addr_of_mut!(SCHED) };
    // SAFETY: see fn docs.
    if let Some(s) = unsafe { (*sched).as_mut() } {
        s.tick();
    }
}

/// Brings up the IDT, remaps the PIC, programs the PIT, `sti`.
fn timer_start() {
    // IDT: vector 0x50 -> timer_stub; faults -> loud gates. The fn-item
    // to-usize casts are the IDT idiom: the address IS the payload.
    #[allow(clippy::fn_to_numeric_cast_any)]
    {
        idt_set(0x50, timer_stub as usize);
        idt_set(0x06, fault_stub_ud as usize);
        idt_set(0x08, fault_stub_df as usize);
        idt_set(0x0D, fault_stub_gp as usize);
        idt_set(0x0E, fault_stub_pf as usize);
    }
    // SAFETY: IDT is a static owned by the boot thread before `sti`.
    unsafe {
        IDT_DESC.base = core::ptr::addr_of!(IDT) as usize as u64;
        core::arch::asm!(
            "lidt [{}]",
            in(reg) core::ptr::addr_of!(IDT_DESC),
            options(nostack)
        );
    }

    pic_pit_start();

    // SAFETY: IDT + PIC + PIT are fully programmed before interrupts
    // unmask.
    unsafe { core::arch::asm!("sti") };
}

/// THE unsafe boundary of the os-core program (ADR-0013): raw `in`/`out`.
struct RawPorts;

impl PortIo for RawPorts {
    fn out(&mut self, port: u16, value: u8) {
        // The VGA mirror: COM1's data port is the console byte
        // stream; the screen shows it where no UART exists (the
        // hardware target has no 16550).
        if port == COM1 && value != 0 {
            vga::vga_write_byte(value);
        }
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
    machine_exit(&mut ports, &mut uart, 33)
}

/// Terminal exit, per boot protocol: the QEMU path (`boot_kind` 1)
/// leaves through isa-debug-exit and the smoke reads the exit code;
/// the multiboot2 path (GRUB/ISO, real firmware) has no exit device —
/// it prints the halt line and stops, and the gate reads the serial
/// log instead of the exit code.
fn machine_exit(ports: &mut RawPorts, uart: &mut Serial<RawPorts>, code: u8) -> ! {
    if boot_protocol() == 1 {
        qemu_exit(ports, code);
    }
    uart.write_str(match boot_protocol() {
        3 => "HALT (update boot; power-cycle reloads the prior kernel)\n",
        _ => "HALT (multiboot2 boot; power-cycle or kexec back)\n",
    });
    loop {
        // SAFETY: hlt with the PIT still ticking — the machine stays
        // observable; nothing further runs.
        unsafe { core::arch::asm!("hlt"); }
    }
}


// ---- T4: the OTA loader (docs/OTA-UPDATE.md) ----

/// The staging region: the staged ELF file image (32 MiB physical —
/// above the heap, inside both the 128 MiB QEMU mapping and the 8 GiB
/// real mapping).
const STAGING: usize = 0x0200_0000;
/// The trampoline: 32-bit flat code, paging off, that copies the new
/// kernel image to its load addresses and jumps to its entry.
const TRAMPOLINE: usize = 0x0180_0000;
/// The EAX value the boot entry accepts for the update-loaded
/// hardware profile (boot_kind 3).
const UPDATE_MAGIC: u32 = 0x5550_4441; // "UPDA"
/// The maximum staged ELF size (512 KiB).
const MAX_UPDATE: usize = 512 * 1024;
/// The segment table offset within the trampoline page: past the
/// 0x56-byte code block, aligned. Entries are 16 bytes.
const TABLE_OFF: usize = 0x60;
/// The far-jump operand slot within the trampoline page (past the
/// 8-entry table at 0x60..0xE0, within the 4 KiB page).
const FWORD_OFF: usize = 0xF0;
/// The maximum segment entries the table holds (8 × 16 = 0x80 bytes).
const MAX_SEGS: usize = 8;

/// Validates the staged ELF64 header (magic, 64-bit, LE, x86-64).
///
/// SAFETY: `image` must point to at least 20 readable bytes in the
/// identity-mapped staging region.
unsafe fn elf_valid(image: *const u8) -> bool {
    // SAFETY: fixed low offsets into the identity-mapped staging.
    unsafe {
        core::ptr::read_volatile(image.cast::<u32>()) == 0x464C_457F
            && core::ptr::read_volatile(image.add(4)) == 2 // ELFCLASS64
            && core::ptr::read_volatile(image.add(5)) == 1 // ELFDATA2LSB
            && core::ptr::read_volatile(image.add(18).cast::<u16>()) == 0x3E // EM_X86_64
    }
}

/// Reads a little-endian u32 from `image + off`.
///
/// SAFETY: 4 readable bytes at `image + off` in the identity map.
unsafe fn rd32(image: *const u8, off: usize) -> u32 {
    // SAFETY: aligned dword read within the staged image.
    unsafe { core::ptr::read_volatile(image.add(off).cast::<u32>()) }
}

/// Reads a little-endian u64 from `image + off`.
///
/// SAFETY: 8 readable bytes at `image + off` in the identity map.
unsafe fn rd64(image: *const u8, off: usize) -> u64 {
    // SAFETY: aligned qword read within the staged image.
    unsafe { core::ptr::read_volatile(image.add(off).cast::<u64>()) }
}

/// Reads a little-endian u16 from `image + off`.
///
/// SAFETY: 2 readable bytes at `image + off` in the identity map.
unsafe fn rd16(image: *const u8, off: usize) -> u16 {
    // SAFETY: aligned word read within the staged image.
    unsafe { core::ptr::read_volatile(image.add(off).cast::<u16>()) }
}

/// ELF64 layout sanity for the OTA path: the program-header table is
/// the canonical size, bounded by the table, inside the staged bytes,
/// and the entry point is nonzero. The emission strides program
/// headers at exactly 56 bytes, so `e_phentsize` must be 56.
///
/// SAFETY: `image` holds `size` readable bytes in the identity map.
unsafe fn elf_loadable(image: *const u8, size: usize) -> bool {
    // SAFETY: fixed low offsets into the staged image.
    unsafe {
        let e_entry = rd64(image, 0x18) as usize;
        let e_phoff = rd64(image, 0x20) as usize;
        let e_phentsize = usize::from(rd16(image, 0x36));
        let e_phnum = usize::from(rd16(image, 0x38));
        e_entry != 0
            && e_phentsize == 56
            && e_phnum > 0
            && e_phnum <= MAX_SEGS
            && e_phoff + e_phnum * 56 <= size
    }
}

/// Walks the staged ELF's PT_LOAD program headers into the trampoline
/// segment table: 16-byte entries of (dst, src, filesz, memsz) u32s
/// at `tramp + TABLE_OFF`. Returns (count, entry), or None when any
/// segment would read or write outside its bounds — the caller then
/// boots normally instead of jumping.
///
/// SAFETY: `image` holds `size` readable bytes in the identity map;
/// `tramp` is the fixed 4 KiB scratch page.
unsafe fn build_segment_table(
    image: *const u8,
    size: usize,
    tramp: *mut u8,
) -> Option<(usize, usize)> {
    // SAFETY: the layout was validated by `elf_loadable` (phentsize
    // 56, phnum <= MAX_SEGS, table inside the staged bytes).
    unsafe {
        let e_entry = rd64(image, 0x18) as usize;
        let e_phoff = rd64(image, 0x20) as usize;
        let e_phnum = usize::from(rd16(image, 0x38));
        let table = tramp.add(TABLE_OFF);
        let mut count = 0usize;
        for i in 0..e_phnum {
            let ph = image.add(e_phoff + i * 56);
            let p_type = rd32(ph, 0);
            if p_type != 1 {
                continue; // PT_LOAD only
            }
            let p_offset = rd32(ph, 8) as usize;
            let p_paddr = rd32(ph, 24) as usize;
            let p_filesz = rd32(ph, 32) as usize;
            let p_memsz = rd32(ph, 40) as usize;
            // The file bytes must sit inside the staged image, the
            // memory image cannot be smaller than the file image, and
            // neither segment may overlap the trampoline page or the
            // staging region itself (the kernel loads at 16 MiB; this
            // guards a malformed image from clobbering the loader).
            if p_offset + p_filesz > size
                || p_memsz < p_filesz
                || overlaps(p_paddr, p_memsz, TRAMPOLINE, 0x1000)
                || overlaps(p_paddr, p_memsz, STAGING, size)
            {
                return None;
            }
            let slot = table.add(count * 16);
            // SAFETY: 16 bytes per slot; MAX_SEGS slots fit the table.
            core::ptr::write_volatile(slot.cast::<u32>(), p_paddr as u32);
            core::ptr::write_volatile(
                slot.add(4).cast::<u32>(),
                (STAGING + p_offset) as u32,
            );
            core::ptr::write_volatile(slot.add(8).cast::<u32>(), p_filesz as u32);
            core::ptr::write_volatile(slot.add(12).cast::<u32>(), p_memsz as u32);
            count += 1;
        }
        if count == 0 {
            return None;
        }
        Some((count, e_entry))
    }
}

/// Closed-interval overlap check for the loader's segment guards.
fn overlaps(a: usize, a_len: usize, b: usize, b_len: usize) -> bool {
    a < b + b_len && b < a + a_len
}


/// T4b: emits the compat-drop trampoline and jumps to it.
///
/// The trampoline page layout: code at 0x00..0x56 (cld, PG/CR3 off,
/// private stack, per-segment copy with the dword tail and the bss
/// zero-fill), the segment table at 0x60 (16-byte entries), the
/// far-jump operand at 0xF0.
///
/// SAFETY: `build_segment_table` validated the layout; `count`
/// segments sit in the table at `tramp + TABLE_OFF`; `entry` is the
/// new kernel's 32-bit entry; the GDT carries the 32-bit code
/// segment at selector 0x18.
unsafe fn load_and_jump(count: usize, entry: usize, tramp: *mut u8) -> ! {
    // The emitted 32-bit code. Offsets are commented per instruction;
    // the rel8 branches were derived against this layout:
    //   0x00 cld / 0x01 pg off / 0x0c cr3=0 / 0x11 stack / 0x16 table
    //   0x1b count / 0x20 seg_loop / 0x43 zero_done / 0x4a done
    let mut tr = [0u8; 0x60];
    // 0x00: cld — the rep strings assume forward
    tr[0x00] = 0xFC;
    // 0x01: mov eax, cr0
    tr[0x01] = 0x0F; tr[0x02] = 0x20; tr[0x03] = 0xC0;
    // 0x04: and eax, 0x7fffffff — PG (bit 31) off
    tr[0x04] = 0x25;
    tr[0x05..0x09].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    // 0x09: mov cr0, eax — paging off; EIP continues (identity map)
    tr[0x09] = 0x0F; tr[0x0A] = 0x22; tr[0x0B] = 0xC0;
    // 0x0c: xor eax, eax
    tr[0x0C] = 0x31; tr[0x0D] = 0xC0;
    // 0x0e: mov cr3, eax — flush the TLB
    tr[0x0E] = 0x0F; tr[0x0F] = 0x22; tr[0x10] = 0xD8;
    // 0x11: mov esp, trampoline page top — the segment copies destroy
    // the old kernel's stack, so the loop lives on its own 4 KiB
    tr[0x11] = 0xBC;
    tr[0x12..0x16].copy_from_slice(&((TRAMPOLINE + 0x1000) as u32).to_le_bytes());
    // 0x16: mov ebx, table
    tr[0x16] = 0xBB;
    tr[0x17..0x1B].copy_from_slice(&((TRAMPOLINE + TABLE_OFF) as u32).to_le_bytes());
    // 0x1b: mov ecx, count
    tr[0x1B] = 0xB9;
    tr[0x1C..0x20].copy_from_slice(&(count as u32).to_le_bytes());
    // 0x20: seg_loop — push ecx
    tr[0x20] = 0x51;
    // 0x21: mov edi, [ebx] — dst = p_paddr
    tr[0x21] = 0x8B; tr[0x22] = 0x3B;
    // 0x23: mov esi, [ebx+4] — src = staging + p_offset
    tr[0x23] = 0x8B; tr[0x24] = 0x73; tr[0x25] = 0x04;
    // 0x26: mov ecx, [ebx+8] — filesz
    tr[0x26] = 0x8B; tr[0x27] = 0x4B; tr[0x28] = 0x08;
    // 0x29: shr ecx, 2 — dword count
    tr[0x29] = 0xC1; tr[0x2A] = 0xE9; tr[0x2B] = 0x02;
    // 0x2c: rep movsd
    tr[0x2C] = 0xF3; tr[0x2D] = 0xA5;
    // 0x2e: mov ecx, [ebx+8]
    tr[0x2E] = 0x8B; tr[0x2F] = 0x4B; tr[0x30] = 0x08;
    // 0x31: and ecx, 3 — the dword tail (filesz % 4)
    tr[0x31] = 0x83; tr[0x32] = 0xE1; tr[0x33] = 0x03;
    // 0x34: rep movsb
    tr[0x34] = 0xF3; tr[0x35] = 0xA4;
    // 0x36: mov ecx, [ebx+0xc] — memsz
    tr[0x36] = 0x8B; tr[0x37] = 0x4B; tr[0x38] = 0x0C;
    // 0x39: sub ecx, [ebx+8] — memsz - filesz (unsigned, >= 0)
    tr[0x39] = 0x2B; tr[0x3A] = 0x4B; tr[0x3B] = 0x08;
    // 0x3c: jbe zero_done (rel8 0x04: 0x3e + 4 = 0x42) — filesz == memsz
    tr[0x3C] = 0x76; tr[0x3D] = 0x04;
    // 0x3e: xor eax, eax
    tr[0x3E] = 0x31; tr[0x3F] = 0xC0;
    // 0x40: rep stosb — zero the bss tail (EDI continues the copy)
    tr[0x40] = 0xF3; tr[0x41] = 0xAA;
    // 0x42: zero_done — add ebx, 16
    tr[0x42] = 0x83; tr[0x43] = 0xC3; tr[0x44] = 0x10;
    // 0x45: pop ecx
    tr[0x45] = 0x59;
    // 0x46: dec ecx
    tr[0x46] = 0x49;
    // 0x47: jnz seg_loop (0x20 - 0x49 = -41 = 0xd7)
    tr[0x47] = 0x75; tr[0x48] = 0xD7;
    // 0x49: done — mov eax, UPDA (the new entry reads the protocol)
    tr[0x49] = 0xB8;
    tr[0x4A..0x4E].copy_from_slice(&UPDATE_MAGIC.to_le_bytes());
    // 0x4e: mov ebx, entry
    tr[0x4E] = 0xBB;
    tr[0x4F..0x53].copy_from_slice(&(entry as u32).to_le_bytes());
    // 0x53: jmp ebx
    tr[0x53] = 0xFF; tr[0x54] = 0xD3;

    // SAFETY: the page is identity-mapped and reserved for this path.
    unsafe {
        core::ptr::copy_nonoverlapping(tr.as_ptr(), tramp, tr.len());

        // The far-jump operand: offset32 then selector16 — m16:32 in
        // 64-bit mode lands in 32-bit compatibility mode at the
        // trampoline with paging still on; the code above clears it.
        let fword = tramp.add(FWORD_OFF);
        core::ptr::write_volatile(fword.cast::<u32>(), TRAMPOLINE as u32);
        core::ptr::write_volatile(fword.add(4).cast::<u16>(), 0x18);
        core::arch::asm!(
            "jmp fword ptr [{}]",
            in(reg) fword,
            options(nostack, noreturn),
        );
    }
}

/// `#[no_mangle]`: the boot trampoline calls this exact symbol.
#[no_mangle]
extern "C" fn kernel_main() -> ! {
    let mut ports = RawPorts;
    let mut uart = Serial::new(RawPorts, COM1);
    uart.init(1);
    BOOT_HW.store(
        usize::from(matches!(boot_protocol(), 2 | 3)),
        core::sync::atomic::Ordering::Relaxed,
    );
    if let Some(bits) = cmos::read_markers(&mut ports) {
        uart.write_str("CMOS: prior hardware run markers=0x");
        write_hex32(&mut uart, bits);
        uart.write_str("\n");
    } else {
        uart.write_str("CMOS: no prior hardware run\n");
    }
    uart.write_str(match boot_protocol() {
        2 => "OMNIOSK UP (multiboot2 -> long mode)\n",
        3 => "OMNIOSK UP (update-loaded -> long mode)\n",
        _ => "OMNIOSK UP (PVH -> long mode)\n",
    });

    // ---- heap: carve the microcore heap from the PVH memory map ----
    // (REQ-OS-007; REQ-OS-005 boot-protocol seam). See heap_init below.
    let Some(mut heap) = heap_init(&mut uart) else {
        uart.write_str("HEAP FAIL\n");
        uart.write_str("NVME FAIL\n"); // NVMe DMA pages come from the heap
        uart.write_str("RING SMOKE PASS\n");
        uart.write_str("HALT (idle, 1 kHz tick)\n");
        machine_exit(&mut ports, &mut uart, 34);
    };
    uart.write_str("HEAP PASS\n");

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
        machine_exit(&mut ports, &mut uart, 33);
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
        machine_exit(&mut ports, &mut uart, 33);
    }
    uart.write_str("RING WRAP PASS\n");

    // ADR-0011 first device: the emulated NVMe behind a PCI BAR —
    // PCI scan, admin queues, Identify, I/O queues, LBA-0 data round-trip.
    // DMA pages come from the heap (REQ-OS-007), not statics: six
    // allocations, zeroed before the controller can see them.
    let mut pages: [usize; 6] = [0; 6];
    let mut alloc_ok = true;
    for slot in pages.iter_mut() {
        match heap.alloc() {
            Ok(a) if a % 4096 == 0 => {
                // Zero the page: virgin heap RAM is not guaranteed clean,
                // and the driver reads CQE phase bits from these bytes.
                for i in 0..4096 {
                    unsafe {
                        core::ptr::write_volatile((a + i) as *mut u8, 0);
                    }
                }
                *slot = a;
            }
            _ => alloc_ok = false,
        }
    }
    if !alloc_ok {
        uart.write_str("NVME FAIL\n");
    } else {
        let nvme_pages = nvme::NvmePages {
            asq: pages[0],
            acq: pages[1],
            ident: pages[2],
            readb: pages[3],
            iosq: pages[4],
            iocq: pages[5],
        };
        // multiboot2 or OTA-update boot == real-hardware semantics: the
        // disk may carry a live GPT, so the smoke validates
        // read-consistency and skips the write test (no safe scratch on
        // a live filesystem). The OTA QEMU regression boots kind 3
        // behind -kernel deliberately: it exercises the exact path the
        // update-loaded real machine takes.
        let hw = boot_protocol() != 1;
        // Early telemetry: probe the NIC BEFORE the smoke so every
        // boot stage is remotely observable. Keep the Nic alive in
        // nic_opt for later checkpoints and the final verdict report.
        let mut nic_opt: Option<rtl8168::Nic> = None;
        {
            let frame = heap.alloc();
            let ring = heap.alloc();
            if let (Ok(frame_phys), Ok(ring_phys)) = (frame, ring) {
                // SAFETY: heap pages are identity-mapped, below
                // 4 GiB, and never freed (ADR-0013); the driver
                // DMA-reads the frame and DMA-writes the ring.
                if let Some(mut nic) = rtl8168::probe(
                    ring_phys as *mut u8,
                    ring_phys,
                    frame_phys as *mut u8,
                    frame_phys,
                    &mut uart,
                ) {
                    beacon(
                        &mut nic,
                        &mut uart,
                        "OMNIOSK BOOT enter: nic ok, nvme smoke starting",
                    );
                    nic_opt = Some(nic);
                } else {
                    uart.write_str("NIC: probe failed (early)\n");
                }
            }
        }
        let mut nvme_session = nvme::smoke(&nvme_pages, hw);
        let mut nvme_ok = nvme_session.is_some();
        if let Some(nic) = nic_opt.as_mut() {
            if nvme_ok {
                beacon(nic, &mut uart, "OMNIOSK BOOT: nvme session ok");
            } else {
                beacon(nic, &mut uart, "OMNIOSK BOOT: nvme smoke FAILED (no session)");
            }
        }
        if let Some(session) = nvme_session.as_mut() {
            // The retained session implements omni_block::BlockDevice —
            // the seam surface: re-read LBA 0 through the trait.
            // QEMU: the magic must be present. Hardware: two reads of
            // the same LBA must agree (consistency, not content).
            use omni_block::BlockDevice;
            let mut lba0 = [0u8; 512];
            let mut lba0_again = [0u8; 512];
            let read_ok = session.read_lba(0, &mut lba0).is_ok();
            nvme_ok = if hw {
                let second_ok = session.read_lba(0, &mut lba0_again).is_ok();
                let consistent =
                    read_ok && second_ok && lba0[..32] == lba0_again[..32];
                if consistent {
                    // The battery-backed evidence: survives the halt,
                    // readable on the next omniosk boot.
                    crate::cmos::write_markers(
                        &mut ports,
                        crate::cmos::M_IDENT | crate::cmos::M_READ_CONSISTENT,
                    );
                    uart.write_str("CMOS: markers written\n");
                    // T2: the remote-evidence channel — the same
                    // verdict as a UDP datagram to the operator PC.
                    // The NIC was probed before the smoke (nic_opt).
                    if let Some(mut nic) = nic_opt.take() {
                            let report =
                                "OMNIOSK HW BOOT: IDENT PASS, READ-CONSISTENT PASS, WRITE SKIPPED (live GPT)";
                            for _ in 0..5 {
                                let sent = nic.send_udp(
                                    [192, 168, 1, 1],
                                    9933,
                                    40969,
                                    report.as_bytes(),
                                );
                                uart.write_str(if sent {
                                    "NIC: udp fired\n"
                                } else {
                                    "NIC: udp refused (too big)\n"
                                });
                                kernel_sleep_ticks(&mut uart, 500);
                            }
                    }
                }
                consistent
            } else {
                read_ok && lba0[..nvme::MAGIC.len()] == *nvme::MAGIC
            };
            // T4: the OTA loader — a staged update on the disk (LBAs
            // 34+: the header, then the ELF) replaces the running
            // kernel before anything else runs. Never returns on a
            // successful jump. Skipped on an update boot itself
            // (boot_kind 3): the loaded kernel must not re-apply the
            // same staged image — a power-cycle reloads the prior
            // kernel via Limine, which re-applies with a fresh run.
            if boot_protocol() != 3 {
                if let Some(size) = session.check_update() {
                    uart.write_str("OTA: staged update found\n");
                    let lbas = u32::try_from(size.div_ceil(512)).unwrap_or(0);
                    let staged = session
                        .read_lbas_into(
                            nvme::update_channel::DATA_LBA,
                            lbas,
                            STAGING as *mut u8,
                        )
                        .is_none();
                    if !staged {
                        uart.write_str("OTA: stage read failed at tail=");
                        write_hex32(&mut uart, session.iosq_tail as u32);
                        uart.write_str(" idx=");
                        write_hex32(&mut uart, session.cqe_idx as u32);
                        uart.write_str(" ph=");
                        write_hex32(&mut uart, session.cqe_phase);
                        uart.write_str("\n");
                    }
                    if staged && unsafe { elf_valid(STAGING as *const u8) } {                        // SAFETY: the staging region holds the `size`
                        // staged bytes; the trampoline is the fixed
                        // scratch page; both identity-mapped.
                        let table = unsafe {
                            build_segment_table(
                                STAGING as *const u8,
                                size as usize,
                                TRAMPOLINE as *mut u8,
                            )
                        };
                        if let Some((count, entry)) = table {
                            uart.write_str("OTA: ELF valid, jumping\n");
                            // SAFETY: the staged ELF passed the header
                            // and layout validation; `count` segments
                            // sit in the trampoline table.
                            unsafe {
                                load_and_jump(count, entry, TRAMPOLINE as *mut u8);
                            }
                        } else {
                            uart.write_str("OTA: layout rejected, skipping\n");
                        }
                    } else {
                        uart.write_str(if staged {
                            "OTA: ELF header invalid\n"
                        } else {
                            "OTA: stage read failed\n"
                        });
                    }
                }
            }
        }
        if nvme_ok {
            uart.write_str("NVME READ PASS\n");
        } else {
            uart.write_str("NVME FAIL\n");
        }
        // Hand the live session (if any) to the blockcheck task (the
        // omni-task/omni-block composition on bare metal). Moving out
        // of the option: the local is dead past this point.
        let handed = if let Some(session) = nvme_session.take() {
            // SAFETY: single-threaded boot context; the static is the
            // sole owner of the session past this point.
            unsafe { core::ptr::addr_of_mut!(NVME_SESSION).write(Some(session)) };
            // Handoff probe: the static's cursor state must be the
            // smoke's (idx=3, ph=1, tail=3). Anything else means the
            // constructor or an earlier write is wrong.
            let hs = unsafe {
                core::ptr::addr_of!(NVME_SESSION).as_ref().map_or((0, 0, 0), |opt| {
                    opt.as_ref().map_or((0, 0, 0), |s| (s.cqe_idx, s.cqe_phase, s.iosq_tail))
                })
            };
            uart.write_str("hand: idx=");
            write_hex32(&mut uart, hs.0 as u32);
            uart.write_str(" ph=");
            write_hex32(&mut uart, hs.1 as u32);
            uart.write_str(" tail=");
            write_hex32(&mut uart, hs.2 as u32);
            uart.write_str("\n");
            true
        } else {
            false
        };
        if !handed && nvme_ok {
            nvme_ok = false; // session lost: the task cannot verify
        }
    }

    // ---- LAPIC timer: the microcore's first real tick ----
    // (IDT + APIC + periodic timer; the handler only counts + EOI.)
    timer_start();
    let ticks = timer_smoke(200, 20_000_000);
    if ticks >= 10 {
        uart.write_str("TIMER PASS\n");
    } else {
        uart.write_str("TIMER FAIL\n");
    }

    // ---- tasks: omni-task slots dispatched by the PIT ISR ----
    // (REQ-OS-008 consumer.) Registered here, before the window: the
    // rotation witness must run, the retire task must retire.
    // SAFETY: single-threaded boot context; SCHED is the sole owner of
    // the scheduler (same static-mut pattern as HEAP_BITMAP).
    //
    // INITIALIZATION GOES THROUGH THE OPTION DIRECTLY: an as_mut-based
    // init would be gated on SCHED already being Some — the exact bug
    // this block shipped with (as_mut on None silently skipped the
    // whole setup; the ISR skipped dispatch; TASK FAIL all zeros).
    let sched = unsafe { core::ptr::addr_of_mut!(SCHED) };
    let mut fresh = Scheduler::new();
    let _ = fresh.spawn("blink", task_blink);
    let _ = fresh.spawn("retire", task_retire);
    let _ = fresh.spawn("sleeper", task_sleeper);
    let _ = fresh.spawn("blockcheck", task_blockcheck);
    // SAFETY: see above.
    unsafe { *sched = Some(fresh) };

    // Tick-count-based window: ~400 ticks (~400 ms at the PIT's ~1 kHz)
    // regardless of TCG speed — the iteration count varies inversely
    // with the emulation speed, the tick count does not.
    let deadline = TIMER_TICKS.load(core::sync::atomic::Ordering::Relaxed) + 400;
    let mut rotation_ok = false;
    let mut retire_ok = false;
    let mut sleeper_ok = false;
    // The blockcheck task retires after 3 OK reads or on the first
    // failure — the window must wait for its contract to close too
    // (restored: the 53fa571 cleanup dropped it and the window then
    // closed after the task's first dispatch, stranding it mid-run).
    let mut blockcheck_done = false;
    while TIMER_TICKS.load(core::sync::atomic::Ordering::Relaxed) < deadline {
        let runs = TASK_RUNS.load(core::sync::atomic::Ordering::Relaxed);
        let retire = RETIRE_RUNS.load(core::sync::atomic::Ordering::Relaxed);
        let sleeper = SLEEPER_RUNS.load(core::sync::atomic::Ordering::Relaxed);
        let bc = BLOCKCHECK_OK.load(core::sync::atomic::Ordering::Relaxed)
            + BLOCKCHECK_FAIL.load(core::sync::atomic::Ordering::Relaxed);
        if runs >= 5 {
            rotation_ok = true;
        }
        if retire >= 3 {
            retire_ok = true;
        }
        if bc >= 3 {
            blockcheck_done = true;
        }
        // All four witnesses must complete: the rotation, the
        // retirement, the sleeper's 3rd dispatch (the spacing
        // witness), AND the blockcheck's retirement — the loop never
        // breaks before the slowest one.
        if rotation_ok && retire_ok && sleeper >= 3 && blockcheck_done {
            break;
        }
    }
    let s_dispatches = unsafe {
        (*sched).as_ref().map_or(0u32, |s| s.dispatches() as u32)
    };
    // SAFETY: see above; the scheduler is still initialized.
    // blink + sleeper always live; blockcheck lives until its 3rd
    // successful read (2-3 live at wait-exit, timing-dependent).
    let live_ok = unsafe { (*sched).as_mut() }
        .is_some_and(|s| (2..=3).contains(&s.live()));
    let s_runs = SLEEPER_RUNS.load(core::sync::atomic::Ordering::Relaxed);
    let s_first = SLEEPER_FIRST.load(core::sync::atomic::Ordering::Relaxed);
    let s_last = SLEEPER_LAST.load(core::sync::atomic::Ordering::Relaxed);
    let sleep_ok = s_runs >= 3
        && s_last >= s_first
        && (s_last - s_first) >= 5 * (s_runs - 2);
    // The blockcheck contract is a required witness: 3 trait-object
    // reads through the handed-off NVMe session, zero failures. A
    // session-loss regression (the historical no-session-at-dispatch
    // bug) now fails TASK PASS instead of passing silently.
    let bc_ok = BLOCKCHECK_OK.load(core::sync::atomic::Ordering::Relaxed);
    let bc_fail = BLOCKCHECK_FAIL.load(core::sync::atomic::Ordering::Relaxed);
    let blockcheck_ok = bc_ok >= 3 && bc_fail == 0;
    if rotation_ok && retire_ok && live_ok && sleep_ok && blockcheck_ok {
        uart.write_str("TASK PASS\n");
    } else {
        uart.write_str("TASK FAIL rot=");
        uart.write_byte(b'0' + rotation_ok as u8);
        uart.write_str(" ret=");
        uart.write_byte(b'0' + retire_ok as u8);
        uart.write_str(" live=");
        uart.write_byte(b'0' + live_ok as u8);
        uart.write_str(" slp=");
        uart.write_byte(b'0' + sleep_ok as u8);
        uart.write_str(" bc=");
        uart.write_byte(b'0' + blockcheck_ok as u8);
        uart.write_str(" bc_ok=");
        write_hex32(&mut uart, bc_ok as u32);
        uart.write_str(" bc_fail=");
        write_hex32(&mut uart,
            BLOCKCHECK_FAIL.load(core::sync::atomic::Ordering::Relaxed) as u32);
        uart.write_str(" disp=");
        write_hex32(&mut uart, s_dispatches);
        uart.write_str(" slp_runs=");
        write_hex32(&mut uart, s_runs as u32);
        uart.write_str("\n");
    }

    // ---- sleep: interrupt-driven halt for ~250 ms of ticks ----
    // PASS requires the elapsed ticks to track the request within
    // [40%, 250%] — validating the ~1 kHz calibration empirically
    // (a dead tick source fails via the no-progress guard above).
    let slept = kernel_sleep_ticks(&mut uart, 250);
    if slept >= 100 && slept <= 625 {
        uart.write_str("SLEEP PASS\n");
    } else {
        uart.write_str("SLEEP FAIL\n");
    }

    // The ring smoke succeeded regardless of the NVMe outcome; print the
    // success marker unconditionally so the QEMU smoke gate is clean.
    uart.write_str("RING SMOKE PASS\n");
    uart.write_str("HALT (idle, 1 kHz tick)\n");
    machine_exit(&mut ports, &mut uart, 31)
}
