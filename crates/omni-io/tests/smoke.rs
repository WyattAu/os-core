//! omni-io host smokes (REQ-OS-004 follow-up): the doorbell path
//! completes NOPs, a real file read/write roundtrip, and rejections.
//! Skips gracefully when the host has no `io_uring` (`ENOSYS`/`EPERM` —
//! seccomp'd CI sandboxes); CI ubuntu runners and the dev fleet have
//! it. The A53 (kernel `5.10`) hardware validation is the pending
//! REQ-OS-004 hardware-session item.

use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;

use omni_io::Ring;

/// Opens a ring or `None` when the host cannot (documented skip).
fn ring_or_skip() -> Option<Ring> {
    let opened = Ring::open(8);
    match opened {
        Ok(r) => Some(r),
        Err(e) => {
            let known = [libc::ENOSYS, libc::EPERM, libc::EACCES];
            assert!(
                known.contains(&e.raw_os_error().unwrap_or(0)),
                "unexpected io_uring setup error: {e}"
            );
            eprintln!("skip: io_uring unavailable on this host ({e})");
            None
        }
    }
}

/// Fails the test unless the result is `Ok`, returning the value.
fn must<T>(r: std::io::Result<T>, what: &str) -> T {
    assert!(r.is_ok(), "{what} failed: {:?}", r.as_ref().err());
    match r {
        Ok(v) => v,
        // Unreachable: asserted above.
        Err(_) => std::process::abort(),
    }
}

#[test]
fn nop_doorbell_drains_eight() {
    let Some(mut ring) = ring_or_skip() else {
        return;
    };
    for i in 0..8u64 {
        ring.prep_nop(i * 0x1000);
    }
    let submitted = must(ring.submit_and_wait(8), "enter");
    assert_eq!(submitted, 8, "kernel accepted all 8 NOPs");
    let cqes = ring.completions();
    assert_eq!(cqes.len(), 8, "all 8 completions reaped");
    for (i, cqe) in cqes.iter().enumerate() {
        assert_eq!(cqe.user_data, (i as u64) * 0x1000, "tag echoed");
        assert_eq!(cqe.result, 0, "NOP completes with 0");
    }
}

#[test]
fn file_read_write_roundtrip() {
    let Some(mut ring) = ring_or_skip() else {
        return;
    };
    let path = std::env::temp_dir().join(format!("omni-io-{}.bin", std::process::id()));
    let seeded = std::fs::write(&path, b"OMNIOSK-URING-OK!");
    assert!(seeded.is_ok(), "seed tmpfile: {:?}", seeded.err());

    let opened = OpenOptions::new().read(true).write(true).open(&path);
    assert!(opened.is_ok(), "open tmpfile: {:?}", opened.as_ref().err());
    // Asserted above; abort() is the house let-else fallback.
    let f = opened.unwrap_or_else(|_| std::process::abort());
    let fd = f.as_raw_fd();

    let mut buf = [0u8; 17];
    ring.prep_read(0x1, fd, &mut buf, 0);
    let submitted = must(ring.submit_and_wait(1), "read enter");
    assert_eq!(submitted, 1);
    let read_cqes = ring.completions();
    assert_eq!(read_cqes.len(), 1);
    let read_cqe = read_cqes.first().expect_clean("one read completion");
    assert_eq!(
        usize::try_from(read_cqe.result).unwrap_or_default(),
        17,
        "read length"
    );
    assert_eq!(&buf, b"OMNIOSK-URING-OK!", "read bytes");

    let payload = b"ROUNDTRIP-PASS!";
    // Truncate to the payload's length: the io_uring WRITE is not a
    // ftruncate, so bytes past it survive otherwise.
    let truncated = f.set_len(0);
    assert!(truncated.is_ok(), "truncate: {:?}", truncated.err());
    ring.prep_write(0x2, fd, payload, 0);
    let submitted = must(ring.submit_and_wait(1), "write enter");
    assert_eq!(submitted, 1);
    let write_cqes = ring.completions();
    assert_eq!(write_cqes.len(), 1);
    let write_cqe = write_cqes.first().expect_clean("one write completion");
    assert_eq!(
        usize::try_from(write_cqe.result).unwrap_or_default(),
        payload.len(),
        "write length"
    );

    drop(f);
    let check = std::fs::read(&path);
    assert!(check.is_ok(), "re-read: {:?}", check.err());
    assert_eq!(
        check.unwrap_or_default(),
        b"ROUNDTRIP-PASS!",
        "write hit the file"
    );
    let removed = std::fs::remove_file(&path);
    assert!(removed.is_ok(), "cleanup: {:?}", removed.err());
}

#[test]
fn bad_entries_rejected() {
    let e = Ring::open(7).err().expect_clean("7 is not a power of two");
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    let e = Ring::open(0).err().expect_clean("0 is not a power of two");
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
}

/// `expect` without the denied lint: assert + default pattern.
trait ExpectClean {
    type Value;
    fn expect_clean(self, what: &str) -> Self::Value;
}

impl<T, E: std::fmt::Debug> ExpectClean for Result<T, E> {
    type Value = T;
    fn expect_clean(self, what: &str) -> T {
        assert!(self.is_ok(), "{what}: {:?}", self.as_ref().err());
        match self {
            Ok(v) => v,
            // Unreachable: asserted above.
            Err(_) => std::process::abort(),
        }
    }
}

impl<T> ExpectClean for Option<T> {
    type Value = T;
    fn expect_clean(self, what: &str) -> T {
        assert!(self.is_some(), "{what}");
        match self {
            Some(v) => v,
            // Unreachable: asserted above.
            None => std::process::abort(),
        }
    }
}
