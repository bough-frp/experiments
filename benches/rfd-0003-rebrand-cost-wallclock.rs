//! Wall-clock time per read of a held value, and per event through a
//! 22-node propagation that reads it once, for each value shape and each way
//! to read it. The groups are the operations (`read`, `snapshot`,
//! `switch_cell`, `sample`, `listener`), the variants the reads (`baseline`
//! is the `unsafe` cast gc-arena uses, an experiment and not Bough code;
//! `restore`, `view` and `borrow` are safe; the module doc says what each
//! is), and the parameter the shape, so each `<op>/<variant>/<shape>`
//! compares with `<op>/baseline/<shape>`.
//!
//! Each iteration is one read or one event; the probe is built once per
//! benchmark, outside what is timed. A hundred benchmarks at about 2.5 s
//! each keep the run near four minutes.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use bough_experiments::rfd_0003_rebrand_cost::{Op, Probe, Shape, Variant};

fn rebrand(c: &mut Criterion) {
    for op in Op::ALL {
        let mut g = c.benchmark_group(op.name());
        g.sample_size(50);
        g.warm_up_time(Duration::from_millis(500));
        g.measurement_time(Duration::from_secs(2));
        for variant in Variant::ALL {
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

criterion_group!(benches, rebrand);
criterion_main!(benches);
