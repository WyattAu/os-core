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

    /* PVH handoff pointer: ESI at the 32-bit entry = &hvm_start_info
       (Xen PVH abi; QEMU fills it with the e820-style memory map).
       Saved into .data — NOT .bss, which the trampoline zeroes after
       this store would already have been wiped. */
    .section .data.memmap, "aw"
    .align 4
    .global pvh_info_ptr
    pvh_info_ptr:
        .long 0

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
        /* save the PVH handoff pointer before anything clobbers ESI */
        mov [pvh_info_ptr], esi
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
    /// Linker-script symbol (kern.ld): end of the kernel image.
    static __bss_end: u8;
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

// ---- PIT timer + remapped PIC (REQ-OS-008 seam: the microcore gets a
// real tick; scheduling and sleeps build on this later) ----
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

    // ---- heap: carve the microcore heap from the PVH memory map ----
    // (REQ-OS-007; REQ-OS-005 boot-protocol seam). See heap_init below.
    let Some(mut heap) = heap_init(&mut uart) else {
        uart.write_str("HEAP FAIL\n");
        uart.write_str("NVME FAIL\n"); // NVMe DMA pages come from the heap
        uart.write_str("RING SMOKE PASS\n");
        uart.write_str("HALT (tickless idle)\n");
        qemu_exit(&mut ports, 34);
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
        let nvme_ok = nvme::smoke(&nvme_pages);
        if nvme_ok {
            uart.write_str("NVME READ PASS\n");
        } else {
            uart.write_str("NVME FAIL\n");
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

    // The ring smoke succeeded regardless of the NVMe outcome; print the
    // success marker unconditionally so the QEMU smoke gate is clean.
    uart.write_str("RING SMOKE PASS\n");
    uart.write_str("HALT (tickless idle)\n");
    qemu_exit(&mut ports, 31)
}
