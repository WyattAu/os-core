//! COM1 serial output — the kernel's first console (ADR-0013).
//!
//! 16550 UART init sequence and byte writes are host-tested against a
//! [`PortIo`] mock. The metal implementation of [`PortIo`] (inline `asm!`
//! port I/O) deliberately does not live here: this workspace denies
//! `unsafe_code`, and the kernel binary introduces that deviation in one
//! audited place when it lands (ADR-0013).

use core::fmt;

/// Port-I/O abstraction so the UART protocol is testable off-metal.
///
/// The real `x86_64` implementation (raw `in`/`out` instructions) arrives
/// with the kernel binary, where `unsafe_code` is explicitly authorized
/// for exactly this module boundary (ADR-0013).
pub trait PortIo {
    /// Writes one byte to the given I/O port.
    fn out(&mut self, port: u16, value: u8);
    /// Reads one byte from the given I/O port.
    fn inp(&mut self, port: u16) -> u8;
}

/// The COM1 base port.
pub const COM1: u16 = 0x3F8;

/// A 16550-style UART on a base port.
pub struct Serial<P: PortIo> {
    ports: P,
    base: u16,
}

impl<P: PortIo> Serial<P> {
    /// Creates the UART without initializing it (see [`Serial::init`]).
    #[must_use]
    pub const fn new(ports: P, base: u16) -> Self {
        Self { ports, base }
    }

    /// Runs the classic 16550 init: disable interrupts, set the divisor
    /// (115200 baud), 8N1, clear FIFOs, enable FIFO.
    pub fn init(&mut self, divisor: u16) {
        let p = self.base;
        self.ports.out(p + 1, 0x00); // disable interrupts
        self.ports.out(p + 3, 0x80); // DLAB on
        self.ports.out(p, (divisor & 0xFF) as u8);
        self.ports.out(p + 1, (divisor >> 8) as u8);
        self.ports.out(p + 3, 0x03); // 8N1, DLAB off
        self.ports.out(p + 2, 0xC7); // FIFO on, clear, 14-byte threshold
        self.ports.out(p + 4, 0x0B); // DTR + RTS + OUT2
    }

    fn transmit_empty(&mut self) -> bool {
        self.ports.inp(self.base + 5) & 0x20 != 0
    }

    /// Blocks until the UART accepts `byte`, then writes it.
    pub fn write_byte(&mut self, byte: u8) {
        while !self.transmit_empty() {}
        self.ports.out(self.base, byte);
    }

    /// Writes a string, mapping newlines to CR-LF.
    pub fn write_str(&mut self, s: &str) {
        for b in s.bytes() {
            match b {
                b'\n' => {
                    self.write_byte(b'\r');
                    self.write_byte(b'\n');
                }
                other => self.write_byte(other),
            }
        }
    }
}

impl<P: PortIo> fmt::Write for Serial<P> {
    fn write_str(&mut self, s: &str) -> Result<(), fmt::Error> {
        Serial::write_str(self, s);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{PortIo, Serial, COM1};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Shared write log the mock records into and the test inspects.
    type WriteLog = Rc<RefCell<Vec<(u16, u8)>>>;

    /// Records every port write in order; reads feed a fixed status byte.
    struct Mock {
        writes: WriteLog,
        status: u8,
    }

    impl PortIo for Mock {
        fn out(&mut self, port: u16, value: u8) {
            self.writes.borrow_mut().push((port, value));
        }

        fn inp(&mut self, _port: u16) -> u8 {
            self.status
        }
    }

    #[test]
    fn init_sequence_is_byte_exact() {
        let writes: WriteLog = Rc::new(RefCell::new(Vec::new()));
        let mut uart = Serial::new(
            Mock {
                writes: Rc::clone(&writes),
                status: 0x20,
            },
            COM1,
        );
        // 115200 baud = divisor 1.
        uart.init(1);
        assert_eq!(
            *writes.borrow(),
            vec![
                (COM1 + 1, 0x00),
                (COM1 + 3, 0x80),
                (COM1, 0x01),
                (COM1 + 1, 0x00),
                (COM1 + 3, 0x03),
                (COM1 + 2, 0xC7),
                (COM1 + 4, 0x0B),
            ]
        );
    }

    #[test]
    fn writes_when_transmit_empty_and_expands_newlines() {
        let writes: WriteLog = Rc::new(RefCell::new(Vec::new()));
        let mut uart = Serial::new(
            Mock {
                writes: Rc::clone(&writes),
                status: 0x20, // THR empty
            },
            COM1,
        );
        uart.write_str("hi\n");
        let data: Vec<u8> = writes
            .borrow()
            .iter()
            .filter(|(port, _)| *port == COM1)
            .map(|(_, v)| *v)
            .collect();
        assert_eq!(data, b"hi\r\n".to_vec());
    }

    #[test]
    fn waits_while_transmit_busy() {
        // First status read reports busy (LSR bit 5 clear), then ready.
        struct BusyThenReady {
            reads: RefCell<u32>,
            writes: WriteLog,
        }
        impl PortIo for BusyThenReady {
            fn out(&mut self, port: u16, value: u8) {
                self.writes.borrow_mut().push((port, value));
            }
            fn inp(&mut self, _p: u16) -> u8 {
                let n = *self.reads.borrow();
                *self.reads.borrow_mut() = n + 1;
                if n == 0 {
                    0x00 // busy
                } else {
                    0x20 // empty
                }
            }
        }
        let writes: WriteLog = Rc::new(RefCell::new(Vec::new()));
        let mut uart = Serial::new(
            BusyThenReady {
                reads: RefCell::new(0),
                writes: Rc::clone(&writes),
            },
            COM1,
        );
        uart.write_byte(b'X');
        // Exactly one byte reached the data port, after the busy wait.
        assert_eq!(*writes.borrow(), vec![(COM1, b'X')]);
    }
}
