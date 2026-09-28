//! Wall-clock times for heap versus mark on quiet regions, RFD 5: one
//! transaction per iteration, over the fixture's fixed cycle of 64
//! events, the same the counts and the instruction counts run.
//!
//! The baseline is RFD 5's depth-first mark and flat loop over the whole
//! marked region. `heap` is a binary heap keyed by height and `bucket` a
//! bucket queue by height, both pushing a node's dependents only when it
//! fires. Groups `ui-small` (253 nodes) and `ui-large` (9,997 nodes) take
//! the filters' pass rate in percent as the parameter; the quiet fraction
//! of the marked region each gives is in the counts. `frame` takes the
//! frame's width, and all of it fires: RFD 5's case for the log factor.
//!
//! Every scheduler runs on its own clone of one fixture, checked first to
//! agree with the others on every event, and warmed for a full cycle so
//! its scratch has grown.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use bough_experiments::rfd_0005_heap_vs_mark_on_quiet_regions::{
    CYCLE, Fixture, PASS, Sched, WIDTH, agree,
};

fn group(c: &mut Criterion, name: &str, params: &[u32], make: fn(u32) -> Fixture) {
    let mut g = c.benchmark_group(name);
    for &p in params {
        let fixture = make(p);
        agree(&fixture, 4 * CYCLE);
        for s in Sched::ALL {
            let mut f = fixture.clone();
            f.steps(s, CYCLE);
            g.bench_function(BenchmarkId::new(s.name(), p), |b| {
                b.iter(|| black_box(f.step(s)))
            });
        }
    }
    g.finish();
}

fn heap_vs_mark(c: &mut Criterion) {
    group(c, "ui-small", &PASS, Fixture::ui_small);
    group(c, "ui-large", &PASS, Fixture::ui_large);
    group(c, "frame", &WIDTH, Fixture::frame);
}

criterion_group! {
    name = benches;
    // 66 benchmarks at about 2.5 s each: under three minutes.
    config = Criterion::default()
        .sample_size(50)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2));
    targets = heap_vs_mark
}
criterion_main!(benches);
