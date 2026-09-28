//! Wall-clock times for Incremental's heights and a bucket queue by them,
//! RFD 5.
//!
//! `<workload>/<variant>/<pass>` runs the workload's 300 transactions (the
//! input event evaluated, the subgraphs built during it, and its commit with
//! the relink check) through one engine. `baseline` is RFD 5's mark and
//! flat loop with the two-way small-side check; `heights` pops firing nodes
//! from a bucket per height, the heights raised at link time; `heap` pops
//! them in the maintained-rank probe's label order from a binary heap. The
//! parameter is the filters' pass rate in percent; the quiet fraction it
//! gives is in the counts. `upkeep/<variant>/<workload>` runs only the
//! builds and commits, for the small-side order (`baseline`) and the
//! heights (`heights`).
//!
//! `<workload>-instant/<variant>/<pass>` runs the same transactions with
//! the nodes they build evaluated in the instant and each moving
//! `switch_cell` reading its new inner then: `baseline` is the mark and flat
//! loop with RFD 5's memoized pull for both cases; `reseat` and `pull` are
//! the heights, raised mid-evaluation, with built nodes below the cursor
//! re-seating it or evaluated at once.
//!
//! Each fixture is first checked to give the same results through every
//! engine. Each iteration runs on a clone of one prepared engine, made in
//! `iter_batched`'s setup and handed back so its drop is not timed.

use std::hint::black_box;
use std::time::Duration;

use criterion::measurement::WallTime;
use criterion::{BatchSize, BenchmarkGroup, BenchmarkId, Criterion, SamplingMode};
use criterion::{criterion_group, criterion_main};

use bough_experiments::rfd_0005_bounded_relink_check::{Checker, Run};
use bough_experiments::rfd_0005_height_queue::{
    HeightQueue, Heights, HeightsPull, HeightsReseat, MarkPull, PASS, WORKLOADS, agree,
    agree_instant,
};
use bough_experiments::rfd_0005_maintained_rank_queue::{
    Engine, Fixture, HeapQueue, MarkOm, Schedule,
};
use bough_experiments::rfd_0005_small_side_order::TwoWay;

fn one<C: Checker, S: Schedule<C>>(
    g: &mut BenchmarkGroup<'_, WallTime>,
    variant: &str,
    fresh: Engine<C, S>,
    f: &Fixture,
) {
    g.bench_function(BenchmarkId::new(variant, f.pass), |b| {
        b.iter_batched(
            || fresh.clone(),
            |mut e| {
                black_box(e.all(black_box(&f.txs), black_box(&f.events)));
                e
            },
            BatchSize::LargeInput,
        )
    });
}

/// An instant engine: anything cloneable that runs the fixture's
/// transactions.
fn instant_one<E: Clone>(
    g: &mut BenchmarkGroup<'_, WallTime>,
    variant: &str,
    fresh: E,
    f: &Fixture,
    all: fn(&mut E, &Fixture) -> u64,
) {
    g.bench_function(BenchmarkId::new(variant, f.pass), |b| {
        b.iter_batched(
            || fresh.clone(),
            |mut e| {
                black_box(all(&mut e, black_box(f)));
                e
            },
            BatchSize::LargeInput,
        )
    });
}

fn upkeep_one<C: Checker>(g: &mut BenchmarkGroup<'_, WallTime>, variant: &str, name: &str) {
    let f = Fixture::new(name, 0);
    let fresh = Run {
        graph: f.graph.clone(),
        checker: C::new(&f.graph),
    };
    g.bench_function(BenchmarkId::new(variant, name), |b| {
        b.iter_batched(
            || fresh.clone(),
            |mut r| {
                black_box(r.all(black_box(&f.txs)));
                r
            },
            BatchSize::LargeInput,
        )
    });
}

fn setup(g: &mut BenchmarkGroup<'_, WallTime>) {
    // One iteration is 300 transactions, tenths of a second: flat sampling
    // keeps ten samples from costing 55 iterations.
    g.sampling_mode(SamplingMode::Flat)
        .sample_size(10)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2));
}

fn height_queue(c: &mut Criterion) {
    for name in WORKLOADS {
        let mut g = c.benchmark_group(name);
        setup(&mut g);
        for pass in PASS {
            let f = Fixture::new(name, pass);
            agree(&f);
            one(&mut g, "baseline", MarkOm::new(&f), &f);
            one(&mut g, "heights", HeightQueue::new(&f), &f);
            one(&mut g, "heap", HeapQueue::new(&f), &f);
        }
        g.finish();
    }
    let mut g = c.benchmark_group("upkeep");
    setup(&mut g);
    for name in WORKLOADS {
        upkeep_one::<TwoWay>(&mut g, "baseline", name);
        upkeep_one::<Heights>(&mut g, "heights", name);
    }
    g.finish();
    for name in WORKLOADS {
        let mut g = c.benchmark_group(format!("{name}-instant"));
        setup(&mut g);
        for pass in PASS {
            let f = Fixture::new(name, pass);
            agree_instant(&f);
            instant_one(&mut g, "baseline", MarkPull::new(&f), &f, |e, f| {
                e.all(&f.txs, &f.events)
            });
            instant_one(&mut g, "reseat", HeightsReseat::new(&f), &f, |e, f| {
                e.all(&f.txs, &f.events)
            });
            instant_one(&mut g, "pull", HeightsPull::new(&f), &f, |e, f| {
                e.all(&f.txs, &f.events)
            });
        }
        g.finish();
    }
}

criterion_group!(benches, height_queue);
criterion_main!(benches);
