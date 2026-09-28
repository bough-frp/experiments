//! Wall-clock times for the small-side order, RFD 5.
//!
//! `moves/<variant>/<workload>` runs a workload's 300 transactions (the
//! subgraphs built during each, then its commit) through one checker:
//! `baseline` is the spike's unbounded upstream walk and `pk` the
//! Pearce–Kelly array, both from `rfd_0005_bounded_relink_check`; `back`
//! searches backward from the new inner and moves what it finds before the
//! switch, and `twoway` searches both ways and moves the side that finishes
//! first, both over an order-maintenance list. Divide by the workload's
//! moves, from the counts, for a time per move. `build/<variant>/lazy`
//! builds every subgraph the `lazy` workload builds during its
//! transactions, with no moves; its ratio to the baseline is what keeping
//! the order adds to building a node. `back` and `twoway` build the same
//! way, so `om` stands for both.
//!
//! Each iteration runs on a clone of one prepared graph and checker, made
//! in `iter_batched`'s setup and handed back so its drop is not timed.
//!
//! `adversarial/<variant>/u<up>-d<down>` runs the adversary: one switch
//! moved 16 times, each to a new inner whose `up` nodes upstream sit after
//! the switch in the order while the switch's `down` nodes downstream sit
//! before it, all accepted; `adversarial-cycle` hangs each new upstream off
//! the switch's downstream, so every move is refused. Divide by 16 for a
//! time per move. Only the moves are timed: before each iteration, untimed,
//! the case is reset, its two moved edges put back and its checker cloned
//! from a fresh one, which costs far less than cloning the graph when the
//! moves are cheap and the graph is large.
//!
//! `back-fresh` and `twoway-fresh`, in `moves` and `adversarial`, are the
//! lists with fresh spacing: a moved set takes half its gap, away from where
//! the next set to that spot goes. Their ratio to `back` and `twoway` is what
//! the spacing saves. (The cyclic adversary never reorders, so they are left
//! out of it.) `mixed/<variant>/s<shared>-n<new>` runs the mixed adversary:
//! 16 moves, each to a new inner that reads a shared upstream of `shared`
//! nodes before the switch in the order and `new` nodes of its own after
//! it, with 1,000 nodes downstream of the switch, timed like the adversary.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};

use bough_experiments::rfd_0005_bounded_relink_check::{
    Baseline, Checker, Pk, Run, Workload, workload,
};
use bough_experiments::rfd_0005_small_side_order::{
    Adversary, Back, BackFresh, MIXED_DOWN, NEW, SHARED, SIDES, TwoWay, TwoWayFresh,
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

fn adversary_one<C: Checker>(
    g: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    variant: &str,
    up: usize,
    down: usize,
    cyclic: bool,
) {
    let case = Adversary::<C>::new(up, down, cyclic);
    time_case(g, variant, format!("u{up}-d{down}"), case);
}

fn mixed_one<C: Checker>(
    g: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    variant: &str,
    shared: usize,
    new: usize,
) {
    let case = Adversary::<C>::mixed(shared, new, MIXED_DOWN);
    time_case(g, variant, format!("s{shared}-n{new}"), case);
}

fn time_case<C: Checker>(
    g: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    variant: &str,
    param: String,
    mut case: Adversary<C>,
) {
    let fresh = case.run.checker.clone();
    g.bench_function(BenchmarkId::new(variant, param), |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                case.reset(&fresh);
                let start = Instant::now();
                black_box(case.all());
                total += start.elapsed();
            }
            total
        })
    });
}

fn adversarial(c: &mut Criterion) {
    for (group, cyclic) in [("adversarial", false), ("adversarial-cycle", true)] {
        let mut g = c.benchmark_group(group);
        g.sample_size(10)
            .warm_up_time(Duration::from_millis(200))
            .measurement_time(Duration::from_millis(500));
        for up in SIDES {
            for down in SIDES {
                adversary_one::<Baseline>(&mut g, "baseline", up, down, cyclic);
                adversary_one::<Pk>(&mut g, "pk", up, down, cyclic);
                adversary_one::<Back>(&mut g, "back", up, down, cyclic);
                adversary_one::<TwoWay>(&mut g, "twoway", up, down, cyclic);
                if !cyclic {
                    adversary_one::<BackFresh>(&mut g, "back-fresh", up, down, cyclic);
                    adversary_one::<TwoWayFresh>(&mut g, "twoway-fresh", up, down, cyclic);
                }
            }
        }
        g.finish();
    }
}

fn mixed(c: &mut Criterion) {
    let mut g = c.benchmark_group("mixed");
    g.sample_size(10)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(500));
    for shared in SHARED {
        for new in NEW {
            mixed_one::<Baseline>(&mut g, "baseline", shared, new);
            mixed_one::<Back>(&mut g, "back", shared, new);
            mixed_one::<TwoWay>(&mut g, "twoway", shared, new);
            mixed_one::<BackFresh>(&mut g, "back-fresh", shared, new);
            mixed_one::<TwoWayFresh>(&mut g, "twoway-fresh", shared, new);
        }
    }
    g.finish();
}

fn order(c: &mut Criterion) {
    let mut g = c.benchmark_group("moves");
    g.sample_size(10).measurement_time(Duration::from_secs(4));
    for name in WORKLOADS {
        let w = workload(name);
        moves_one::<Baseline>(&mut g, "baseline", name, &w);
        moves_one::<Pk>(&mut g, "pk", name, &w);
        moves_one::<Back>(&mut g, "back", name, &w);
        moves_one::<TwoWay>(&mut g, "twoway", name, &w);
        moves_one::<BackFresh>(&mut g, "back-fresh", name, &w);
        moves_one::<TwoWayFresh>(&mut g, "twoway-fresh", name, &w);
    }
    g.finish();

    let mut g = c.benchmark_group("build");
    g.sample_size(20).measurement_time(Duration::from_secs(4));
    let w = workload("lazy");
    build_one::<Baseline>(&mut g, "baseline", &w);
    build_one::<Pk>(&mut g, "pk", &w);
    build_one::<Back>(&mut g, "om", &w);
    g.finish();
}

criterion_group!(benches, order, adversarial, mixed);
criterion_main!(benches);
