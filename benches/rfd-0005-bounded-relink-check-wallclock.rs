//! Wall-clock times for the bounded relink check, RFD 5.
//!
//! `moves/<variant>/<workload>` runs a workload's 300 transactions (the
//! subgraphs built during each, then its commit) through one checker:
//! `baseline` is the spike's unbounded upstream walk, `pk` the maintained
//! Pearce–Kelly order, `summaries` the per-subgraph reachability bitsets.
//! Divide by the workload's moves, from the counts, for a time per move.
//! `build/<variant>/lazy` builds every subgraph the `lazy` workload builds
//! during its transactions, with no moves; its ratio to the baseline is
//! what keeping the order or the summaries adds to building a node.
//!
//! Each iteration runs on a clone of one prepared graph and checker, made
//! in `iter_batched`'s setup and handed back so its drop is not timed.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};

use bough_experiments::rfd_0005_bounded_relink_check::{
    Baseline, Checker, Pk, Run, Summaries, Workload, workload,
};

const WORKLOADS: [&str; 4] = ["settled", "mixed", "churn", "lazy"];

fn moves_one<C: Checker>(
    g: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    variant: &str,
    name: &str,
    w: &Workload,
) {
    let fresh = Run::<C>::new(w);
    g.bench_function(BenchmarkId::new(variant, name), |b| {
        b.iter_batched(
            || fresh.clone(),
            |mut run| {
                black_box(run.all(black_box(&w.txs)));
                run
            },
            BatchSize::LargeInput,
        )
    });
}

fn build_one<C: Checker>(
    g: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    variant: &str,
    w: &Workload,
) {
    let fresh = Run::<C>::new(w);
    let builds = w.builds();
    g.bench_function(BenchmarkId::new(variant, "lazy"), |b| {
        b.iter_batched(
            || fresh.clone(),
            |mut run| {
                for x in black_box(&builds) {
                    run.build(x);
                }
                run
            },
            BatchSize::LargeInput,
        )
    });
}

fn relink(c: &mut Criterion) {
    let mut g = c.benchmark_group("moves");
    g.sample_size(10).measurement_time(Duration::from_secs(4));
    for name in WORKLOADS {
        let w = workload(name);
        moves_one::<Baseline>(&mut g, "baseline", name, &w);
        moves_one::<Pk>(&mut g, "pk", name, &w);
        moves_one::<Summaries>(&mut g, "summaries", name, &w);
    }
    g.finish();

    let mut g = c.benchmark_group("build");
    g.sample_size(20).measurement_time(Duration::from_secs(4));
    let w = workload("lazy");
    build_one::<Baseline>(&mut g, "baseline", &w);
    build_one::<Pk>(&mut g, "pk", &w);
    build_one::<Summaries>(&mut g, "summaries", &w);
    g.finish();
}

criterion_group!(benches, relink);
criterion_main!(benches);
