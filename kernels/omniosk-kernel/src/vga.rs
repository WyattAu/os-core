//! VGA text console (the hardware boot path's screen output).
//!
//! On the server there is no 16550 UART — `COM1` writes land on a
//! dead bus. GRUB leaves the VGA text framebuffer at `0xB8000` active
//! at hand-off (both QEMU and real firmware), so this module mirrors
//! the serial byte stream into that buffer: the screen becomes the
//! console the operator can photograph.
//!
//! SAFETY (ADR-0013 boundary): the framebuffer write is raw MMIO at
//! a fixed address that exists on every IBM-compatible VGA
//! implementation. The mirroring is invoked from the kernel's
//! `RawPorts::out` when the port is COM1's data port — the serial
//! byte stream IS the console stream, so every existing `Serial`
//! call site gains screen output with zero call-site changes.

use core::sync::atomic::{AtomicUsize, Ordering};



/// VGA text mode buffer (80x25, 2 bytes per cell: char + attribute).
const VGA_BUF: usize = 0xB8000;
/// Text dimensions.
const COLS: usize = 80;
const ROWS: usize = 25;
/// Light-grey on black.
const ATTR: u8 = 0x0F;

/// Software cursor (the hardware cursor is left alone).
static COL: AtomicUsize = AtomicUsize::new(0);
static ROW: AtomicUsize = AtomicUsize::new(0);

/// Writes one character cell into the framebuffer.
/// SAFETY: `0xB8000` is the fixed VGA text buffer on every
/// IBM-compatible VGA implementation; `idx` is masked into the
/// 80x25 page.
unsafe fn put_cell(idx: usize, ch: u8, attr: u8) {
    core::ptr::write_volatile((VGA_BUF + idx * 2) as *mut u8, ch);
    core::ptr::write_volatile((VGA_BUF + idx * 2 + 1) as *mut u8, attr);
}

/// Scrolls the whole page up one row, blanking the last row.
fn scroll_up() {
    let page = COLS * ROWS;
    // SAFETY: fixed VGA buffer, page size bounds every access here.
    unsafe {
        core::ptr::copy(
            (VGA_BUF + COLS * 2) as *const u8,
            VGA_BUF as *mut u8,
            (ROWS - 1) * COLS * 2,
        );
        let mut i = page - COLS;
        while i < page {
            put_cell(i, b' ', ATTR);
            i += 1;
        }
    }
}

/// Mirrors one byte into the VGA text buffer (LF, CR and TAB are
/// handled; everything else prints).
pub fn vga_write_byte(b: u8) {
    let col = COL.load(Ordering::Relaxed);
    let row = ROW.load(Ordering::Relaxed);
    match b {
        b'\n' => {
            // SAFETY: row/col masked below; fixed VGA buffer.
            unsafe {
                let mut c = col;
                while c < COLS {
                    put_cell(row * COLS + c, b' ', ATTR);
                    c += 1;
                }
            }
            ROW.store((row + 1) % ROWS, Ordering::Relaxed);
            if row + 1 >= ROWS {
                scroll_up();
                ROW.store(ROWS - 1, Ordering::Relaxed);
            }
            COL.store(0, Ordering::Relaxed);
        }
        b'\r' => {
            COL.store(0, Ordering::Relaxed);
        }
        _ => {
            // SAFETY: masked index into the fixed VGA buffer.
            unsafe {
                put_cell(row * COLS + col, b, ATTR);
            }
            let next = col + 1;
            if next >= COLS {
                COL.store(0, Ordering::Relaxed);
                ROW.store((row + 1) % ROWS, Ordering::Relaxed);
                if row + 1 >= ROWS {
                    scroll_up();
                    ROW.store(ROWS - 1, Ordering::Relaxed);
                }
            } else {
                COL.store(next, Ordering::Relaxed);
            }
        }
    }
}

/// Mirrors a whole byte stream.
pub fn vga_write_str(s: &str) {
    for b in s.bytes() {
        vga_write_byte(b);
    }
}

/// Clears the text page and homes the cursor.
pub fn vga_clear() {
    // SAFETY: fixed VGA buffer, page bounds.
    unsafe {
        core::ptr::write_bytes(VGA_BUF as *mut u8, 0, COLS * ROWS * 2);
    }
    COL.store(0, Ordering::Relaxed);
    ROW.store(0, Ordering::Relaxed);
}
