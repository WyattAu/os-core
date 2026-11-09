//! Ring throughput baselines (REQ-OS-001) — compared against
//! `bench/baseline.tsv` by `scripts/bench-budget.sh`.

// `criterion_group!` generates an undocumented entry point; the missing-docs
// gate is aimed at our API surface, not benchmark harness plumbing.
#![allow(missing_docs)]

use criterion::{criterion_group, criterion_main, Criterion};
use omni_ring::{mpmc::MpmcRing, spsc::SpscRing};
use std::hint::black_box;

fn spsc_pingpong(c: &mut Criterion) {
    c.bench_function("spsc_pingpong_cap64", |b| {
        let ring: SpscRing<64> = SpscRing::new();
        let mut seq: u64 = 0;
        b.iter(|| {
            let _ = ring.try_push(black_box(seq));
            if let Some(v) = ring.try_pop() {
                seq = black_box(v);
            }
        });
    });
}

fn mpmc_pingpong(c: &mut Criterion) {
    c.bench_function("mpmc_pingpong_cap64", |b| {
        let ring: MpmcRing<u64> = MpmcRing::new(64);
        b.iter(|| {
            let _ = ring.push(0);
            black_box(ring.pop());
        });
    });
}

criterion_group!(benches, spsc_pingpong, mpmc_pingpong);
criterion_main!(benches);
