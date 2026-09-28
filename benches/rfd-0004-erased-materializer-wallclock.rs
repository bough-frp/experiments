//! Wall-clock time per event through a chain of three adapters,
//! `input.map(f).filter(p).snapshot(cell, g).hold(b, 0)`, with the chain
//! handed to its materializer four ways: generic over the chain type
//! (`baseline`, today's fusion), erased to one boxed closure (`boxed`),
//! erased to a state pointer and a step fn pointer (`fnptr`), and
//! normalized into one flat (state, step) node (`flat`).
//!
//! Each iteration is one transaction carrying one event: the send, every
//! node's eval through its ops table, and the commit. The graph is built
//! once per benchmark, outside what is timed. The erased designs pay one
//! more indirect call per event than `baseline`; the question is whether
//! that stays within a few nanoseconds.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use bough_experiments::rfd_0004_erased_materializer::{Design, ThreeAdapters};

fn event(c: &mut Criterion) {
    let mut g = c.benchmark_group("event");
    g.sample_size(100);
    g.measurement_time(Duration::from_secs(10));
    for design in Design::ALL {
        let mut chain = ThreeAdapters::new(design);
        g.bench_function(BenchmarkId::new(design.name(), 3), |b| {
            let mut x = 0u64;
            b.iter(|| {
                x += 1;
                chain.event(black_box(x));
            })
        });
        black_box(chain.value());
    }
    g.finish();
}

criterion_group!(benches, event);
criterion_main!(benches);
