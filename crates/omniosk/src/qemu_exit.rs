//! QEMU `isa-debug-exit` device — CI-able kernel exits (ADR-0013).
//!
//! Writing `(code << 1) | 1` to port `0xF4` makes QEMU exit with status
//! `code`, which lets `scripts/qemu-omniosk.sh` assert kernel success in
//! automation without a display.

use crate::serial::PortIo;

/// The isa-debug-exit port.
pub const DEBUG_EXIT_PORT: u16 = 0xF4;

/// The byte QEMU expects on [`DEBUG_EXIT_PORT`] for exit status `code`.
#[must_use]
pub const fn exit_byte(code: u8) -> u8 {
    (code << 1) | 1
}

/// Exits QEMU with `code` via the isa-debug-exit device.
///
/// On `x86_64` (the only gate target) the port write ends the machine; the
/// diverging loop afterwards exists so the function's type is `!` even if
/// the device is absent.
pub fn qemu_exit<P: PortIo>(ports: &mut P, code: u8) -> ! {
    ports.out(DEBUG_EXIT_PORT, exit_byte(code));
    #[cfg(target_arch = "x86_64")]
    {
        loop {
            core::hint::spin_loop();
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    unreachable!("no non-x86 gate target exists today")
}

#[cfg(test)]
mod tests {
    use super::{exit_byte, DEBUG_EXIT_PORT};

    #[test]
    fn exit_encoding_matches_isa_debug_exit() {
        assert_eq!(exit_byte(0), 1);
        assert_eq!(exit_byte(33), (33 << 1) | 1);
        assert_eq!(exit_byte(0x7F), (0x7F << 1) | 1);
    }

    #[test]
    fn debug_exit_port_is_the_documented_0xf4() {
        assert_eq!(DEBUG_EXIT_PORT, 0xF4);
    }
}
