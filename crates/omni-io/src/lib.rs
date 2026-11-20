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
/// `io_uring_setup` flag: a kernel thread submits from the SQ.
const IORING_SETUP_SQPOLL: u32 = 1 << 1;

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

/// usize → u16 for values the API bounds at or below the table size.
fn cu16(v: usize) -> u16 {
    u16::try_from(v).unwrap_or_default()
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
    /// Registered-buffer table size (0 = none registered).
    buf_count: usize,
    /// Registered buffer base addresses (liburing also stamps `addr`
    /// on fixed ops; some kernel paths require it).
    buf_addrs: Vec<u64>,
    /// The ring is SQPOLL: a kernel thread submits; `enter` only reaps.
    sqpoll: bool,
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
        Self::open_with(entries, |_| {})
    }

    /// Opens a SQPOLL ring: a kernel thread submits from the SQ on its
    /// own (sleeping `idle_ms` between passes), so the hot path never
    /// traps into the syscall at all — the A53's 5,129 ns/op batch
    /// mode. Needs registered buffers on 5.10-adjacent kernels;
    /// unprivileged since 5.13. Failures are host policy (`EPERM`,
    /// `EINVAL`) — callers skip, not panic.
    ///
    /// # Errors
    ///
    /// [`io::Error`] from `io_uring_setup` verbatim.
    pub fn open_sqpoll(entries: usize, idle_ms: u32) -> io::Result<Self> {
        Self::open_with(entries, |p| {
            p.flags = IORING_SETUP_SQPOLL;
            p.sq_thread_idle = idle_ms;
        })
    }

    fn open_with(entries: usize, configure: impl FnOnce(&mut Params)) -> io::Result<Self> {
        if !entries.is_power_of_two() || entries > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "entries must be a power of two in 1..=4096",
            ));
        }
        let mut params = Params::zeroed();
        configure(&mut params);
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
            buf_count: 0,
            buf_addrs: Vec::new(),
            sqpoll: params.flags & IORING_SETUP_SQPOLL != 0,
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

    /// Registers `bufs` with the kernel (`IORING_REGISTER_BUFFERS`):
    /// the fixed opcodes then address buffers by index, eliding the
    /// per-op pin — the HFT discipline for hot re-submission of the
    /// same pool. `bufs` must outlive the ring (the kernel pins the
    /// pages; the registered table dies with the fd at Drop).
    ///
    /// # Errors
    ///
    /// [`io::Error`] from `io_uring_register` verbatim; `InvalidInput`
    /// for an empty slice.
    pub fn register_buffers<const N: usize>(&mut self, bufs: &mut [[u8; N]]) -> io::Result<()> {
        if bufs.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at least one buffer is required",
            ));
        }
        let iovs: Vec<libc::iovec> = bufs
            .iter_mut()
            .map(|b| libc::iovec {
                iov_base: b.as_mut_ptr().cast(),
                iov_len: b.len(),
            })
            .collect();
        // SAFETY: fd is the open ring; (op=0 REGISTER_BUFFERS, iovec
        // array, count) matches io_uring_register(2).
        let ret = unsafe {
            libc::syscall(
                libc::SYS_io_uring_register,
                self.fd,
                0usize, // IORING_REGISTER_BUFFERS
                iovs.as_ptr(),
                iovs.len(),
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        self.buf_addrs = bufs.iter().map(|b| b.as_ptr().addr() as u64).collect();
        self.buf_count = bufs.len();
        Ok(())
    }

    /// Queues a `READ_FIXED` (opcode 4): reads into the registered
    /// buffer at `buf_index` — no per-op address pin. Requires
    /// [`Ring::register_buffers`] first; returns `false` (queuing
    /// nothing) for an unregistered index.
    pub fn prep_read_fixed(
        &mut self,
        user_data: u64,
        fd: RawFd,
        buf_index: usize,
        len: u32,
        offset: u64,
    ) -> bool {
        if buf_index >= self.buf_count {
            return false;
        }
        let slot = (self.tail & self.sq_mask) as usize;
        // SAFETY: masked slot inside the SQE array. Offsets: opcode@0,
        // flags@1, ioprio@2, fd@4, off@8, addr@16 (0: table-resolved),
        // len@24, user_data@32, buf_index@40.
        unsafe {
            let sqe = self.sqes_map.0.add(slot * SQE_SIZE);
            sqe.add(0).write_volatile(4u8); // opcode: READ_FIXED
            sqe.add(1).write_volatile(0u8);
            sqe.cast::<u16>().add(1).write_volatile(0u16);
            sqe.cast::<i32>().add(1).write_volatile(fd);
            sqe.cast::<u64>().add(1).write_volatile(offset);
            sqe.cast::<u64>().add(2).write_volatile(
                self.buf_addrs
                    .first()
                    .and_then(|_| self.buf_addrs.get(buf_index))
                    .copied()
                    .unwrap_or_default(),
            );
            sqe.cast::<u32>().add(6).write_volatile(len);
            sqe.cast::<u64>().add(4).write_volatile(user_data);
            sqe.cast::<u16>().add(20).write_volatile(cu16(buf_index));
        }
        self.tail = self.tail.wrapping_add(1);
        self.pending += 1;
        true
    }

    /// Queues a `WRITE_FIXED` (opcode 5): writes `len` bytes from the
    /// registered buffer at `buf_index`. Requires
    /// [`Ring::register_buffers`] first.
    pub fn prep_write_fixed(
        &mut self,
        user_data: u64,
        fd: RawFd,
        buf_index: usize,
        len: u32,
        offset: u64,
    ) -> bool {
        if buf_index >= self.buf_count {
            return false;
        }
        let slot = (self.tail & self.sq_mask) as usize;
        // SAFETY: masked slot inside the SQE array, as in
        // `prep_read_fixed`.
        unsafe {
            let sqe = self.sqes_map.0.add(slot * SQE_SIZE);
            sqe.add(0).write_volatile(5u8); // opcode: WRITE_FIXED
            sqe.add(1).write_volatile(0u8);
            sqe.cast::<u16>().add(1).write_volatile(0u16);
            sqe.cast::<i32>().add(1).write_volatile(fd);
            sqe.cast::<u64>().add(1).write_volatile(offset);
            sqe.cast::<u64>().add(2).write_volatile(
                self.buf_addrs
                    .first()
                    .and_then(|_| self.buf_addrs.get(buf_index))
                    .copied()
                    .unwrap_or_default(),
            );
            sqe.cast::<u32>().add(6).write_volatile(len);
            sqe.cast::<u64>().add(4).write_volatile(user_data);
            sqe.cast::<u16>().add(20).write_volatile(cu16(buf_index));
        }
        self.tail = self.tail.wrapping_add(1);
        self.pending += 1;
        true
    }

    /// Rings the doorbell: submits everything queued and waits until at
    /// least `wait` completions are reaped. Returns the number of SQEs
    /// the kernel accepted.
    ///
    /// # Errors
    ///
    /// [`io::Error`] from `io_uring_enter` verbatim.
    pub fn submit_and_wait(&mut self, wait: usize) -> io::Result<usize> {
        // SQPOLL rings: the kernel thread submits from the shared page;
        // the doorbell side of `enter` must stay zero (EINVAL otherwise).
        let to_submit = if self.sqpoll { 0 } else { self.pending };
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
        if self.sqpoll {
            // The kernel thread owns submission; the app-side pending
            // count is void the moment the tail page is published.
            self.pending = 0;
        } else {
            self.pending -= to_submit.min(self.pending);
        }
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
