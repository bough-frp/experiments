//! Wall-clock time per write of a held value, and per event through a
//! 22-node propagation that writes it once, for each value shape and each
//! way to write it. The groups are the operations, prefixed `write_` so they
//! don't share a directory with the read probe's (`write_store`,
//! `write_update`, `write_hold`, `write_accumulate`, `write_switch_cell`),
//! the variants the writes (`baseline` is the `unsafe` cast gc-arena uses,
//! an experiment and not Bough code; `rebrand`, `view`, `owned` and `borrow`
//! are safe, `borrow` on updates only; the module doc says what each is),
//! and the parameter the shape, so each `<op>/<variant>/<shape>` compares
//! with `<op>/baseline/<shape>`.
//!
//! Each iteration is one write or one event; the probe is built once per
//! benchmark, outside what is timed. 110 benchmarks at about 2.5 s each
//! keep the run near five minutes.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use bough_experiments::rfd_0003_rebrand_write_cost::{Op, Probe, Shape, Variant};

fn rebrand_write(c: &mut Criterion) {
    for op in Op::ALL {
        let mut g = c.benchmark_group(format!("write_{}", op.name()));
        g.sample_size(50);
        g.warm_up_time(Duration::from_millis(500));
        g.measurement_time(Duration::from_secs(2));
        for variant in Variant::ALL.into_iter().filter(|v| v.applies(op)) {
            for shape in Shape::ALL {
                let mut probe = Probe::new(op, variant, shape);
                g.bench_function(BenchmarkId::new(variant.name(), shape.name()), |b| {
                    b.iter(|| black_box(probe.run(1)))
                });
            }
        }
        g.finish();
    }
}

criterion_group!(benches, rebrand_write);
criterion_main!(benches);
