//! REQ-OS-004 spike: doorbell-driven submission economics on Linux.
//!
//! Compares three ingestion idioms against `/dev/zero` (pure kernel path,
//! no device noise):
//!
//! 1. `syscall`  — one `read(2)` per op (the classic trap-per-operation)
//! 2. `uring-1`  — one SQE + one doorbell (`submit_and_wait(1)`) per op
//! 3. `uring-batch` — BATCH SQEs, one doorbell, BATCH CQEs reaped
//!
//! The question the goals corpus cares about (§11, §16: doorbell-driven
//! execution, direct queue mapping) is how much of the per-op cost is the
//! trap vs the work, and how batching amortizes the doorbell. Output is
//! TSV: `mode  ops  ns_per_op  ops_per_sec`.
//!
//! Not part of the workspace gates (spikes/ is excluded); run via
//! `scripts/io-spike.sh`.

use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::time::Instant;

const BUF: usize = 512;
const OPS: usize = 20_000;
const BATCH: usize = 64;

fn bench_syscall(ops: usize) -> std::io::Result<f64> {
    let mut f = File::open("/dev/zero")?;
    let mut buf = [0u8; BUF];
    let t = Instant::now();
    for _ in 0..ops {
        f.read_exact(&mut buf)?;
    }
    let elapsed = t.elapsed().as_nanos() as f64;
    Ok(elapsed / ops as f64)
}

fn bench_uring(ops: usize, batch: usize) -> Result<f64, Box<dyn std::error::Error>> {
    use io_uring::{opcode, types, IoUring};

    let mut ring = IoUring::new((batch as u32 * 2).max(4))?;
    let file = File::open("/dev/zero")?;
    let fd = types::Fd(file.as_raw_fd());
    // One buffer per in-flight slot; a slot is reused only after its batch
    // is fully reaped below, so no buffer is ever referenced by two CQEs.
    let mut bufs: Vec<Vec<u8>> = (0..batch).map(|_| vec![0u8; BUF]).collect();

    let mut offset: u64 = 0;
    let mut completed: usize = 0;
    let t = Instant::now();

    while completed < ops {
        let mut pushed = 0usize;
        while pushed < batch.min(ops - completed) {
            let entry = opcode::Read::new(fd, bufs[pushed].as_mut_ptr(), BUF as u32)
                .offset(offset)
                .build()
                .user_data(pushed as u64);
            // SAFETY: `bufs[pushed]` outlives the operation; the slot is
            // only reused after every completion of this batch is reaped.
            unsafe {
                ring.submission()
                    .push(&entry)
                    .map_err(|_| "submission queue full")?;
            }
            offset += BUF as u64;
            pushed += 1;
        }

        ring.submit_and_wait(pushed)?;

        let mut reaped = 0usize;
        for cqe in ring.completion() {
            let n = cqe.result();
            if n < 0 {
                return Err(format!("/dev/zero read failed with {}", n).into());
            }
            reaped += 1;
        }
        completed += reaped;
    }

    let elapsed = t.elapsed().as_nanos() as f64;
    Ok(elapsed / completed as f64)
}

fn main() {
    let ops: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(OPS);
    const REPS: usize = 7;

    println!("mode\tops\tmedian_ns_per_op\tmin_ns_per_op");

    let mut samples: Vec<f64> = Vec::new();
    let mut report = |mode: &str, samples: &mut Vec<f64>| {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = samples.len() / 2;
        let median = samples[mid];
        println!("{mode}\t{ops}\t{median:.1}\t{:.1}", samples[0]);
    };

    samples.clear();
    for _ in 0..REPS {
        match bench_syscall(ops) {
            Ok(ns) => samples.push(ns),
            Err(e) => eprintln!("syscall: skipped ({e})"),
        }
    }
    if !samples.is_empty() {
        report("syscall", &mut samples);
    }

    samples.clear();
    for _ in 0..REPS {
        match bench_uring(ops, 1) {
            Ok(ns) => samples.push(ns),
            Err(e) => eprintln!("uring-1: skipped ({e})"),
        }
    }
    if !samples.is_empty() {
        report("uring-1", &mut samples);
    }

    samples.clear();
    for _ in 0..REPS {
        match bench_uring(ops, BATCH) {
            Ok(ns) => samples.push(ns),
            Err(e) => eprintln!("uring-batch: skipped ({e})"),
        }
    }
    if !samples.is_empty() {
        report(&format!("uring-batch{BATCH}"), &mut samples);
    }
}
