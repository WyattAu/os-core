//! # omni-io — `Linux` `io_uring` seam over raw syscalls (K2, REQ-OS-004)
//!
//! Doorbell-driven I/O: the submission queue is a descriptor ring, the
//! completion queue is a result ring, and `io_uring_enter` is the only
//! trap. This crate owns the wire format directly — `io_uring_setup` /
//! `io_uring_enter` / `mmap` over raw syscalls — the same
//! own-the-bytes posture as omni-sbe's SBE codec, so the ring layouts
//! are not delegated to a third-party binding whose version skew
//! becomes our skew.
//!
//! The cursor arithmetic (`tail & mask`, per-ring injectivity of the
//! ring positions) is exactly the algebra machine-checked in
//! `proofs/Proofs.lean` (`spsc_slot_bounded`, `spsc_slot_injective`,
//! `spsc_inflight_distinct`) — the K1 primitives applied to the K2
//! doorbell surface.
//!
//! Posture (ADR-0016): `Linux`-only, `std` (the rings are mmap'd — no
//! `no_std` story without a hero allocator), kernel 5.10+ layouts read
//! back from the device (`io_uring_params` offsets, never hardcoded),
//! single-producer/submitter v0.1. The unsafe surface is this crate's
//! syscall/mmap seam; every block carries a SAFETY comment and the
//! ADR is the deviation record for `unsafe_code`.
//!
//! Scope (v0.1.0): ring setup/teardown, `NOP`, `READ`, `WRITE`,
//! submit-and-wait, completion drain. SQPOLL (the A53 batch numbers),
//! registered buffers, and multi-submitter hardening are the next
//! tranche and REQ-tracked.

#![allow(unsafe_code)]
// ADR-0016: the syscall/mmap seam IS the surface
// Ring accesses are volatile u8-pointer field writes; the SQE array is
// 64-byte aligned in practice, but the alignment lint is syntactic on
// `*mut u8` casts — one scoped allow with the ADR reference.
#![allow(clippy::cast_ptr_alignment)]

use std::io;
use std::os::unix::io::RawFd;

/// syscall numbers via `libc` (`x86_64` `io_uring`_setup=425, enter=426 —
/// libc carries the arch-correct numbers).
use libc::{SYS_io_uring_enter, SYS_io_uring_setup};

/// mmap placement of the three shared regions (the kernel's contract).
const IORING_OFF_SQ_RING: libc::off_t = 0x0;
const IORING_OFF_CQ_RING: libc::off_t = 0x800_0000;
const IORING_OFF_SQES: libc::off_t = 0x1000_0000;

/// `io_uring_enter` flag: also reap completions.
const IORING_ENTER_GETEVENTS: libc::c_uint = 1;

/// Completion-queue entry: 16 bytes, `user_data | res | flags`.
const CQE_SIZE: usize = 16;
/// Submission-queue entry: 64 bytes.
const SQE_SIZE: usize = 64;

/// `io_uring_params`, laid out per the 5.10+ contract; the device
/// fills the entries and the per-ring field offsets at `setup`.
#[repr(C)]
#[allow(dead_code)] // the struct models the kernel ABI: field-complete by design
struct Params {
    sq_entries: u32,
    cq_entries: u32,
    flags: u32,
    sq_thread_cpu: u32,
    sq_thread_idle: u32,
    features: u32,
    wq_fd: u32,
    resv: [u32; 3],
    sq_off: SqOffsets,
    cq_off: CqOffsets,
}

/// The SQ ring's field offsets (note: `array`, no `cqes`).
#[repr(C)]
#[allow(dead_code)] // kernel ABI: field-complete by design
struct SqOffsets {
    head: u32,
    tail: u32,
    ring_mask: u32,
    ring_entries: u32,
    flags: u32,
    dropped: u32,
    array: u32,
    resv1: u32,
    user_addr: u64,
}

/// The CQ ring's field offsets (note: `overflow` + `cqes`, no `array`).
#[repr(C)]
#[allow(dead_code)] // kernel ABI: field-complete by design
struct CqOffsets {
    head: u32,
    tail: u32,
    ring_mask: u32,
    ring_entries: u32,
    overflow: u32,
    cqes: u32,
    flags: u32,
    resv1: u32,
    user_addr: u64,
}

impl Params {
    fn zeroed() -> Self {
        // SAFETY: all-zero params is the documented input (flags = 0);
        // every field of this repr(C) struct is valid at bit-pattern 0.
        unsafe { core::mem::zeroed() }
    }
}

/// One completed operation: the caller's tag and the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cqe {
    /// The value passed at submission time.
    pub user_data: u64,
    /// Bytes transferred, or `-errno` on failure.
    pub result: i32,
}

fn with_ctx(e: &io::Error, what: &'static str) -> io::Error {
    io::Error::new(e.kind(), format!("{what}: {e}"))
}

/// A single `io_uring` instance: one mmap'd SQ, one mmap'd CQ, the SQE
/// array, and the doorbell (`io_uring_enter`).
pub struct Ring {
    fd: RawFd,
    entries: usize,
    sq_map: (*mut u8, usize),
    cq_map: (*mut u8, usize),
    sqes_map: (*mut u8, usize),
    sq_off: SqOffsets,
    cq_off: CqOffsets,
    sq_mask: u32,
    cq_mask: u32,
    /// SQEs prepared but not yet submitted.
    pending: u32,
    /// The ring tail (next SQE slot); the source of truth is this
    /// local, mirrored into the shared page at submit.
    tail: u32,
}

impl Ring {
    /// Opens a ring with `entries` slots (power of two, 1..=4096 per
    /// the kernel's limit).
    ///
    /// # Errors
    ///
    /// [`io::Error`] from `io_uring_setup`/`mmap` verbatim — including
    /// `ENOSYS` (kernel without `io_uring`) and `EPERM` (seccomp'd);
    /// `InvalidInput` for a bad `entries`.
    pub fn open(entries: usize) -> io::Result<Self> {
        if !entries.is_power_of_two() || entries > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "entries must be a power of two in 1..=4096",
            ));
        }
        let mut params = Params::zeroed();
        // SAFETY: `params` is a valid, zeroed repr(C) `io_uring`_params;
        // the kernel writes its view into it. On error the raw syscall
        // returns -errno, mapped to io::Error below.
        let fd = unsafe {
            libc::syscall(
                SYS_io_uring_setup,
                entries as libc::c_ulong,
                core::ptr::addr_of_mut!(params),
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = fd as RawFd;

        let sq_len = params.sq_off.array as usize + params.sq_entries as usize * 4;
        let cq_len = params.cq_off.cqes as usize + params.cq_entries as usize * CQE_SIZE;
        let sqes_len = params.sq_entries as usize * SQE_SIZE;

        // SAFETY: the three mmaps use the fd + fixed region offsets
        // the `io_uring` contract defines; lengths cover exactly the
        // ring the kernel reported. MAP_POPULATE prefaults so first
        // touch never stalls a latency path.
        let sq_ptr = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                sq_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_POPULATE,
                fd,
                IORING_OFF_SQ_RING,
            )
        };
        let cq_ptr = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                cq_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_POPULATE,
                fd,
                IORING_OFF_CQ_RING,
            )
        };
        let sqes_ptr = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                sqes_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_POPULATE,
                fd,
                IORING_OFF_SQES,
            )
        };
        for (p, _len, what) in [
            (sq_ptr, sq_len, "sq"),
            (cq_ptr, cq_len, "cq"),
            (sqes_ptr, sqes_len, "sqes"),
        ] {
            if p == libc::MAP_FAILED {
                // SAFETY: any earlier successful mmaps are torn down
                // before the error propagates; fd is owned by Self
                // only after all three succeed.
                unsafe { Self::teardown(fd, sq_ptr, sq_len, cq_ptr, cq_len, sqes_ptr, sqes_len) };
                return Err(with_ctx(&io::Error::last_os_error(), what));
            }
        }

        let ring = Self {
            fd,
            entries: params.sq_entries as usize,
            sq_map: (sq_ptr.cast::<u8>(), sq_len),
            cq_map: (cq_ptr.cast::<u8>(), cq_len),
            sqes_map: (sqes_ptr.cast::<u8>(), sqes_len),
            sq_off: params.sq_off,
            cq_off: params.cq_off,
            sq_mask: params.sq_entries - 1,
            cq_mask: params.cq_entries - 1,
            pending: 0,
            tail: 0,
        };
        // Identity mapping: ring position i carries SQE i. The masked
        // indexing is the Lean-proven algebra (`spsc_slot_bounded`):
        // `tail & mask` can never leave the ring.
        // SAFETY: array region holds `entries` u32 slots inside the
        // mmap'd SQ ring; i < entries.
        unsafe {
            // The array region is byte-aligned in the ring page; the
            // u32 stores are done as unaligned volatile writes.
            #[allow(clippy::cast_ptr_alignment)] // unaligned u32 writes by contract
            let array = ring.sq_map.0.add(ring.sq_off.array as usize).cast::<u32>();
            for i in 0..entries {
                array
                    .add(i)
                    .write_volatile(u32::try_from(i).unwrap_or_default());
            }
        }
        Ok(ring)
    }

    /// The ring's entry count.
    #[must_use]
    pub fn entries(&self) -> usize {
        self.entries
    }

    /// Queues a `NOP` (opcode 0): exercises the doorbell path with no
    /// device. `user_data` is echoed back on the completion.
    pub fn prep_nop(&mut self, user_data: u64) {
        let slot = (self.tail & self.sq_mask) as usize;
        // SAFETY: slot < entries (masked index), so the SQE write stays
        // inside the mmap'd SQE array. Offsets: opcode@0, flags@1,
        // ioprio@2, fd@4, off@8, addr@16, len@24, user_data@32.
        unsafe {
            let sqe = self.sqes_map.0.add(slot * SQE_SIZE);
            sqe.add(0).write_volatile(0u8); // opcode: NOP
            sqe.add(1).write_volatile(0u8); // flags
            sqe.cast::<u16>().add(1).write_volatile(0u16); // ioprio
            sqe.cast::<i32>().add(1).write_volatile(-1); // fd: none
            sqe.cast::<u64>().add(1).write_volatile(0); // off
            sqe.cast::<u64>().add(2).write_volatile(0); // addr
            sqe.cast::<u32>().add(6).write_volatile(0); // len
            sqe.cast::<u64>().add(4).write_volatile(user_data);
        }
        self.tail = self.tail.wrapping_add(1);
        self.pending += 1;
    }

    /// Queues a `READ` (opcode 22, kernel 5.6+): read at `offset` into
    /// `buf`. The buffer must outlive the call until the matching
    /// completion is drained — `submit_and_wait` gives that.
    pub fn prep_read(&mut self, user_data: u64, fd: RawFd, buf: &mut [u8], offset: u64) {
        let slot = (self.tail & self.sq_mask) as usize;
        let len = libc::c_uint::try_from(buf.len()).unwrap_or_default();
        // SAFETY: masked slot inside the SQE array, as in `prep_nop`.
        unsafe {
            let sqe = self.sqes_map.0.add(slot * SQE_SIZE);
            sqe.add(0).write_volatile(22u8); // opcode: READ
            sqe.add(1).write_volatile(0u8);
            sqe.cast::<u16>().add(1).write_volatile(0u16);
            sqe.cast::<i32>().add(1).write_volatile(fd);
            sqe.cast::<u64>().add(1).write_volatile(offset);
            sqe.cast::<u64>()
                .add(2)
                .write_volatile(buf.as_mut_ptr().addr() as u64);
            sqe.cast::<u32>().add(6).write_volatile(len);
            sqe.cast::<u64>().add(4).write_volatile(user_data);
        }
        self.tail = self.tail.wrapping_add(1);
        self.pending += 1;
    }

    /// Queues a `WRITE` (opcode 23): write `buf` at `offset`. The
    /// buffer must outlive the call until the completion is drained.
    pub fn prep_write(&mut self, user_data: u64, fd: RawFd, buf: &[u8], offset: u64) {
        let slot = (self.tail & self.sq_mask) as usize;
        let len = libc::c_uint::try_from(buf.len()).unwrap_or_default();
        // SAFETY: masked slot inside the SQE array, as in `prep_nop`.
        unsafe {
            let sqe = self.sqes_map.0.add(slot * SQE_SIZE);
            sqe.add(0).write_volatile(23u8); // opcode: WRITE
            sqe.add(1).write_volatile(0u8);
            sqe.cast::<u16>().add(1).write_volatile(0u16);
            sqe.cast::<i32>().add(1).write_volatile(fd);
            sqe.cast::<u64>().add(1).write_volatile(offset);
            sqe.cast::<u64>()
                .add(2)
                .write_volatile(buf.as_ptr().addr() as u64);
            sqe.cast::<u32>().add(6).write_volatile(len);
            sqe.cast::<u64>().add(4).write_volatile(user_data);
        }
        self.tail = self.tail.wrapping_add(1);
        self.pending += 1;
    }

    /// Rings the doorbell: submits everything queued and waits until at
    /// least `wait` completions are reaped. Returns the number of SQEs
    /// the kernel accepted.
    ///
    /// # Errors
    ///
    /// [`io::Error`] from `io_uring_enter` verbatim.
    pub fn submit_and_wait(&mut self, wait: usize) -> io::Result<usize> {
        let to_submit = self.pending;
        // Publish the tail before the doorbell (the volatile store plus
        // the syscall's implicit barrier orders the SQE writes).
        // SAFETY: sq ring tail field, inside the mmap'd SQ ring.
        unsafe {
            self.sq_map
                .0
                .add(self.sq_off.tail as usize)
                .cast::<u32>()
                .write_volatile(self.tail);
        }
        // SAFETY: fd is the open ring; args match `io_uring`_enter(2):
        // (fd, to_submit, min_complete, flags, sig=NULL, sigsz=0).
        let ret = unsafe {
            libc::syscall(
                SYS_io_uring_enter,
                self.fd,
                libc::c_ulong::from(to_submit),
                libc::c_ulong::try_from(wait).unwrap_or_default(),
                libc::c_ulong::from(IORING_ENTER_GETEVENTS),
                0usize,
                0usize,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        let accepted = usize::try_from(ret).unwrap_or_default();
        self.pending -= to_submit.min(self.pending);
        Ok(accepted)
    }

    /// Drains every completion currently in the CQ and publishes the
    /// consumed head back to the kernel.
    pub fn completions(&mut self) -> Vec<Cqe> {
        // SAFETY: cq head/tail fields inside the mmap'd CQ ring.
        let head = unsafe {
            core::ptr::read_volatile(self.cq_map.0.add(self.cq_off.head as usize).cast::<u32>())
        };
        let tail = unsafe {
            core::ptr::read_volatile(self.cq_map.0.add(self.cq_off.tail as usize).cast::<u32>())
        };
        let mut out = Vec::with_capacity(tail.wrapping_sub(head) as usize);
        let mut h = head;
        while h != tail {
            let slot = (h & self.cq_mask) as usize;
            // SAFETY: masked slot inside the mmap'd CQ ring; each CQE
            // is 16 bytes at cq_off.cqes.
            unsafe {
                let cqe = self
                    .cq_map
                    .0
                    .add(self.cq_off.cqes as usize + slot * CQE_SIZE);
                let user_data = cqe.cast::<u64>().read_volatile();
                let result = cqe.cast::<i32>().add(2).read_volatile();
                out.push(Cqe { user_data, result });
            }
            h = h.wrapping_add(1);
        }
        // SAFETY: consumed head publish, inside the mmap'd CQ ring.
        unsafe {
            self.cq_map
                .0
                .add(self.cq_off.head as usize)
                .cast::<u32>()
                .write_volatile(h);
        }
        out
    }
}

impl Ring {
    // SAFETY: tears down every successfully mapped region and closes
    // the fd; used by Drop and by the partial-open error path.
    unsafe fn teardown(
        fd: RawFd,
        sq: *mut libc::c_void,
        sq_len: usize,
        cq: *mut libc::c_void,
        cq_len: usize,
        sqes: *mut libc::c_void,
        sqes_len: usize,
    ) {
        if sq != libc::MAP_FAILED {
            libc::munmap(sq.cast(), sq_len);
        }
        if cq != libc::MAP_FAILED {
            libc::munmap(cq.cast(), cq_len);
        }
        if sqes != libc::MAP_FAILED {
            libc::munmap(sqes.cast(), sqes_len);
        }
        libc::close(fd);
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        // SAFETY: each region was mapped with exactly these lengths.
        unsafe {
            Self::teardown(
                self.fd,
                self.sq_map.0.cast(),
                self.sq_map.1,
                self.cq_map.0.cast(),
                self.cq_map.1,
                self.sqes_map.0.cast(),
                self.sqes_map.1,
            );
        }
    }
}
