//! Instruction counts for the sweep-cost probe: one collection of the arena
//! at a thousand, ten thousand and a hundred thousand slots, with a tenth
//! and nine tenths of it surviving, split into its mark, its sweep and its
//! prune, against one transaction over one screen (21 nodes, the same at
//! every size) and a pass that reads each slot's liveness and stamp once. The wall-clock bench says why the transaction is the
//! baseline.
//!
//! Each fixture is built in setup, outside what is counted: fresh for the
//! collection and the mark, marked for the sweep, swept for the prune, and
//! collected for the transaction and the pass. Each bench hands the
//! fixture back, so its drop runs in teardown, uncounted.

use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use bough_experiments::rfd_0003_sweep_cost::Fixture;

fn fresh(slots: usize, live: usize) -> Fixture {
    Fixture::new(slots, live)
}

fn marked(slots: usize, live: usize) -> Fixture {
    Fixture::new(slots, live).marked()
}

fn swept(slots: usize, live: usize) -> Fixture {
    Fixture::new(slots, live).swept()
}

fn collected(slots: usize, live: usize) -> Fixture {
    Fixture::new(slots, live).collected()
}

fn drop_fixture<T>(out: (T, Fixture)) {
    black_box(out.0);
}

#[library_benchmark]
#[benches::live10(args = [(1_000, 10), (10_000, 10), (100_000, 10)], setup = collected, teardown = drop_fixture)]
#[benches::live90(args = [(1_000, 90), (10_000, 90), (100_000, 90)], setup = collected, teardown = drop_fixture)]
fn baseline(mut f: Fixture) -> (i64, Fixture) {
    let input = f.kept.input;
    (black_box(f.arena.transaction(input, black_box(7))), f)
}

#[library_benchmark]
#[benches::live10(args = [(1_000, 10), (10_000, 10), (100_000, 10)], setup = fresh, teardown = drop_fixture)]
#[benches::live90(args = [(1_000, 90), (10_000, 90), (100_000, 90)], setup = fresh, teardown = drop_fixture)]
fn collect(mut f: Fixture) -> (usize, Fixture) {
    (black_box(f.arena.collect()), f)
}

#[library_benchmark]
#[benches::live10(args = [(1_000, 10), (10_000, 10), (100_000, 10)], setup = fresh, teardown = drop_fixture)]
#[benches::live90(args = [(1_000, 90), (10_000, 90), (100_000, 90)], setup = fresh, teardown = drop_fixture)]
fn mark(mut f: Fixture) -> (usize, Fixture) {
    (black_box(f.arena.mark_roots()), f)
}

#[library_benchmark]
#[benches::live10(args = [(1_000, 10), (10_000, 10), (100_000, 10)], setup = marked, teardown = drop_fixture)]
#[benches::live90(args = [(1_000, 90), (10_000, 90), (100_000, 90)], setup = marked, teardown = drop_fixture)]
fn sweep(mut f: Fixture) -> (usize, Fixture) {
    (black_box(f.arena.sweep()), f)
}

#[library_benchmark]
#[benches::live10(args = [(1_000, 10), (10_000, 10), (100_000, 10)], setup = swept, teardown = drop_fixture)]
#[benches::live90(args = [(1_000, 90), (10_000, 90), (100_000, 90)], setup = swept, teardown = drop_fixture)]
fn prune(mut f: Fixture) -> ((), Fixture) {
    f.arena.prune();
    ((), f)
}

#[library_benchmark]
#[benches::live10(args = [(1_000, 10), (10_000, 10), (100_000, 10)], setup = collected, teardown = drop_fixture)]
#[benches::live90(args = [(1_000, 90), (10_000, 90), (100_000, 90)], setup = collected, teardown = drop_fixture)]
fn pass(f: Fixture) -> (usize, Fixture) {
    (black_box(f.arena.touch_each_slot()), f)
}

library_benchmark_group!(
    name = sweep_cost;
    benchmarks = baseline, collect, mark, sweep, prune, pass
);

main!(library_benchmark_groups = sweep_cost);
