//! Instruction counts for a queue keyed by a maintained order, RFD 5.
//!
//! Each benchmark runs a workload's 300 transactions through one engine:
//! the input event evaluated, the subgraphs built during the transaction,
//! and its commit with the relink check. `baseline` is RFD 5 as written, the
//! mark and flat loop with the upstream walk per move; `mark_with_om` the
//! same mark and loop with the small-side order as the check; `radix` and
//! `heap` pop firing nodes in the order's label order from a radix heap and
//! a binary heap. The parameter is the filters' pass rate in percent; the
//! quiet fraction of the marked region it gives is in the counts. Divide by
//! 300 for a transaction.
//!
//! `upkeep_*` runs the same transactions' builds and commits with no events,
//! for the walk and the small-side order: what the check and the order's
//! upkeep cost apart from evaluation.
//!
//! The `flat` group runs the same transactions with the scheduler reading
//! flat adjacency, one edge array per direction with a slice per node,
//! patched as nodes are built and switches move: `flat_mark_with_om`,
//! `flat_heap`, and `flat_heights`, Incremental's heights and a bucket per
//! height, with `heights`, the height-queue probe's engine over the nested
//! lists, beside it. `flat_upkeep` is `upkeep_order` and the heights'
//! upkeep, each with and without the flat arrays following: the flat
//! layout's own cost is the difference.
//!
//! The fixture, the graph and the checker's initial order are built in
//! setup, and everything is handed back so its drop isn't counted.

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use bough_experiments::rfd_0005_bounded_relink_check::{Baseline, Checker, Run};
use bough_experiments::rfd_0005_height_queue::{HeightQueue, Heights};
use bough_experiments::rfd_0005_maintained_rank_queue::{
    Engine, Fixture, Flat, FlatEngine, FlatHeapQueue, FlatHeightQueue, FlatMarkOm, FlatSchedule,
    HeapQueue, MarkOm, RadixQueue, Schedule, Walked,
};
use bough_experiments::rfd_0005_small_side_order::TwoWay;

fn engine<C: Checker, S: Schedule<C>>(name: &str, pass: u32) -> (Engine<C, S>, Fixture) {
    let f = Fixture::new(name, pass);
    (Engine::new(&f), f)
}

fn walked(name: &str, pass: u32) -> (Walked, Fixture) {
    engine(name, pass)
}

fn mark_om(name: &str, pass: u32) -> (MarkOm, Fixture) {
    engine(name, pass)
}

fn radix_queue(name: &str, pass: u32) -> (RadixQueue, Fixture) {
    engine(name, pass)
}

fn heap_queue(name: &str, pass: u32) -> (HeapQueue, Fixture) {
    engine(name, pass)
}

fn height_queue(name: &str, pass: u32) -> (HeightQueue, Fixture) {
    engine(name, pass)
}

fn flat<C: Checker, S: FlatSchedule<C>>(name: &str, pass: u32) -> (FlatEngine<C, S>, Fixture) {
    let f = Fixture::new(name, pass);
    (FlatEngine::new(&f), f)
}

fn flat_mark_om(name: &str, pass: u32) -> (FlatMarkOm, Fixture) {
    flat(name, pass)
}

fn flat_heap_queue(name: &str, pass: u32) -> (FlatHeapQueue, Fixture) {
    flat(name, pass)
}

fn flat_height_queue(name: &str, pass: u32) -> (FlatHeightQueue, Fixture) {
    flat(name, pass)
}

fn run_flat<C: Checker, S: FlatSchedule<C>>(
    (mut e, f): (FlatEngine<C, S>, Fixture),
) -> (FlatEngine<C, S>, Fixture, u64) {
    let digest = e.all(black_box(&f.txs), black_box(&f.events));
    (e, f, digest)
}

fn run<C: Checker, S: Schedule<C>>(
    (mut e, f): (Engine<C, S>, Fixture),
) -> (Engine<C, S>, Fixture, u64) {
    let digest = e.all(black_box(&f.txs), black_box(&f.events));
    (e, f, digest)
}

fn upkeep<C: Checker>(name: &str) -> (Run<C>, Fixture) {
    let f = Fixture::new(name, 0);
    (
        Run {
            graph: f.graph.clone(),
            checker: C::new(&f.graph),
        },
        f,
    )
}

fn upkeep_walk(name: &str) -> (Run<Baseline>, Fixture) {
    upkeep(name)
}

fn upkeep_om(name: &str) -> (Run<TwoWay>, Fixture) {
    upkeep(name)
}

fn upkeep_heights_nested(name: &str) -> (Run<Heights>, Fixture) {
    upkeep(name)
}

fn upkeep_flat<C: Checker>(name: &str) -> (Run<C>, Flat, Fixture) {
    let (r, f) = upkeep(name);
    (r, Flat::new(&f.graph), f)
}

fn upkeep_flat_om(name: &str) -> (Run<TwoWay>, Flat, Fixture) {
    upkeep_flat(name)
}

fn upkeep_flat_heights(name: &str) -> (Run<Heights>, Flat, Fixture) {
    upkeep_flat(name)
}

fn checks_flat<C: Checker>(
    (mut r, mut flat, f): (Run<C>, Flat, Fixture),
) -> (Run<C>, Flat, Fixture, usize) {
    let refused = flat.all(&mut r, black_box(&f.txs));
    (r, flat, f, refused)
}

fn checks<C: Checker>((mut r, f): (Run<C>, Fixture)) -> (Run<C>, Fixture, usize) {
    let refused = r.all(black_box(&f.txs));
    (r, f, refused)
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = walked)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = walked)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = walked)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = walked)]
fn baseline(input: (Walked, Fixture)) -> (Walked, Fixture, u64) {
    black_box(run(input))
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = mark_om)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = mark_om)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = mark_om)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = mark_om)]
fn mark_with_om(input: (MarkOm, Fixture)) -> (MarkOm, Fixture, u64) {
    black_box(run(input))
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = radix_queue)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = radix_queue)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = radix_queue)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = radix_queue)]
fn radix(input: (RadixQueue, Fixture)) -> (RadixQueue, Fixture, u64) {
    black_box(run(input))
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = heap_queue)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = heap_queue)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = heap_queue)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = heap_queue)]
fn heap(input: (HeapQueue, Fixture)) -> (HeapQueue, Fixture, u64) {
    black_box(run(input))
}

#[library_benchmark]
#[benches::all(args = ["settled", "mixed", "churn", "lazy"], setup = upkeep_walk)]
fn upkeep_baseline(input: (Run<Baseline>, Fixture)) -> (Run<Baseline>, Fixture, usize) {
    black_box(checks(input))
}

#[library_benchmark]
#[benches::all(args = ["settled", "mixed", "churn", "lazy"], setup = upkeep_om)]
fn upkeep_order(input: (Run<TwoWay>, Fixture)) -> (Run<TwoWay>, Fixture, usize) {
    black_box(checks(input))
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = flat_mark_om)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = flat_mark_om)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = flat_mark_om)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = flat_mark_om)]
fn flat_mark_with_om(input: (FlatMarkOm, Fixture)) -> (FlatMarkOm, Fixture, u64) {
    black_box(run_flat(input))
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = flat_heap_queue)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = flat_heap_queue)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = flat_heap_queue)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = flat_heap_queue)]
fn flat_heap(input: (FlatHeapQueue, Fixture)) -> (FlatHeapQueue, Fixture, u64) {
    black_box(run_flat(input))
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = height_queue)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = height_queue)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = height_queue)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = height_queue)]
fn heights(input: (HeightQueue, Fixture)) -> (HeightQueue, Fixture, u64) {
    black_box(run(input))
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = flat_height_queue)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = flat_height_queue)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = flat_height_queue)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = flat_height_queue)]
fn flat_heights(input: (FlatHeightQueue, Fixture)) -> (FlatHeightQueue, Fixture, u64) {
    black_box(run_flat(input))
}

#[library_benchmark]
#[benches::all(args = ["settled", "mixed", "churn", "lazy"], setup = upkeep_flat_om)]
fn upkeep_flat_order(input: (Run<TwoWay>, Flat, Fixture)) -> (Run<TwoWay>, Flat, Fixture, usize) {
    black_box(checks_flat(input))
}

#[library_benchmark]
#[benches::all(args = ["settled", "mixed", "churn", "lazy"], setup = upkeep_heights_nested)]
fn upkeep_heights(input: (Run<Heights>, Fixture)) -> (Run<Heights>, Fixture, usize) {
    black_box(checks(input))
}

#[library_benchmark]
#[benches::all(args = ["settled", "mixed", "churn", "lazy"], setup = upkeep_flat_heights)]
fn upkeep_flat_heights(
    input: (Run<Heights>, Flat, Fixture),
) -> (Run<Heights>, Flat, Fixture, usize) {
    black_box(checks_flat(input))
}

library_benchmark_group!(
    name = transactions;
    benchmarks = baseline, mark_with_om, radix, heap
);

library_benchmark_group!(
    name = upkeep;
    benchmarks = upkeep_baseline, upkeep_order
);

library_benchmark_group!(
    name = flat;
    benchmarks = flat_mark_with_om, flat_heap, heights, flat_heights
);

library_benchmark_group!(
    name = flat_upkeep;
    benchmarks = upkeep_flat_order, upkeep_heights, upkeep_flat_heights
);

main!(
    library_benchmark_groups = transactions,
    upkeep,
    flat,
    flat_upkeep
);
