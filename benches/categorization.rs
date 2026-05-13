use splog::categorize::extract;

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("extract long tag", |b| b.iter(|| extract(black_box("2015-07-29 19:04:29,071 - WARN  [SendWorker:188978561024:QuorumCnxManager$SendWorker@688] - Send worker leaving thread"))));
    c.bench_function("extract multiple tags", |b| b.iter(|| extract(black_box("2026-05-02T09:43:45.729516 - INFO - Main - Server - Cluster0 - Cargo Profile: debug"))));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
