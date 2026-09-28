//! Instruction counts for Incremental's heights and a bucket queue by
//! them, RFD 5.
//!
//! Each benchmark runs a workload's 300 transactions through one engine:
//! the input event evaluated, the subgraphs built during the transaction,
//! and its commit with the relink check. `baseline` is RFD 5's mark and
//! flat loop with the two-way small-side check; `heights` pops firing nodes
//! from a bucket per height, the heights raised at link time; `heap` pops
//! them in the maintained-rank probe's label order from a binary heap. The
//! parameter is the filters' pass rate in percent; the quiet fraction of
//! the marked region it gives is in the counts. Divide by 300 for a
//! transaction.
//!
//! `upkeep_*` runs the same transactions' builds and commits with no
//! events, for the small-side order and the heights: what the check and its
//! upkeep cost apart from evaluation.
//!
//! `instant::*` runs the same transactions with the two cases RFD 5 names
//! evaluated in the instant: the nodes a transaction builds run in it, and
//! a `switch_cell` that moves reads its new inner's post-instant value.
//! `instant_baseline` is the mark and flat loop with RFD 5's memoized pull
//! for both; `instant_reseat` and `instant_pull` are the heights, raised
//! mid-evaluation for the new inners, with built nodes below the cursor
//! re-seating it or evaluated at once. Their counts are in the `-counts-pull`
//! results. `instant_unforced` is the mark and pull without the construct
//! point forced into the mark, marking from the input and the moving
//! `switch_cell`s only; its counts are in the `-counts-unforced` results.
//!
//! The fixture, the graph and the checker's initial order or heights are
//! built in setup, and everything is handed back so its drop isn't counted.

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use bough_experiments::rfd_0005_bounded_relink_check::{Checker, Run};
use bough_experiments::rfd_0005_height_queue::{
    HeightQueue, Heights, HeightsPull, HeightsReseat, MarkPull, MarkUnforced,
};
use bough_experiments::rfd_0005_maintained_rank_queue::{
    Engine, Fixture, HeapQueue, MarkOm, Schedule,
};
use bough_experiments::rfd_0005_small_side_order::TwoWay;

fn engine<C: Checker, S: Schedule<C>>(name: &str, pass: u32) -> (Engine<C, S>, Fixture) {
    let f = Fixture::new(name, pass);
    (Engine::new(&f), f)
}

fn mark_om(name: &str, pass: u32) -> (MarkOm, Fixture) {
    engine(name, pass)
}

fn height_queue(name: &str, pass: u32) -> (HeightQueue, Fixture) {
    engine(name, pass)
}

fn heap_queue(name: &str, pass: u32) -> (HeapQueue, Fixture) {
    engine(name, pass)
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

fn upkeep_om(name: &str) -> (Run<TwoWay>, Fixture) {
    upkeep(name)
}

fn upkeep_heights(name: &str) -> (Run<Heights>, Fixture) {
    upkeep(name)
}

fn checks<C: Checker>((mut r, f): (Run<C>, Fixture)) -> (Run<C>, Fixture, usize) {
    let refused = r.all(black_box(&f.txs));
    (r, f, refused)
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = mark_om)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = mark_om)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = mark_om)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = mark_om)]
fn baseline(input: (MarkOm, Fixture)) -> (MarkOm, Fixture, u64) {
    black_box(run(input))
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
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = heap_queue)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = heap_queue)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = heap_queue)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = heap_queue)]
fn heap(input: (HeapQueue, Fixture)) -> (HeapQueue, Fixture, u64) {
    black_box(run(input))
}

#[library_benchmark]
#[benches::all(args = ["settled", "mixed", "churn", "lazy"], setup = upkeep_om)]
fn upkeep_baseline(input: (Run<TwoWay>, Fixture)) -> (Run<TwoWay>, Fixture, usize) {
    black_box(checks(input))
}

#[library_benchmark]
#[benches::all(args = ["settled", "mixed", "churn", "lazy"], setup = upkeep_heights)]
fn upkeep_height(input: (Run<Heights>, Fixture)) -> (Run<Heights>, Fixture, usize) {
    black_box(checks(input))
}

fn mark_pull(name: &str, pass: u32) -> (MarkPull, Fixture) {
    let f = Fixture::new(name, pass);
    (MarkPull::new(&f), f)
}

fn heights_reseat(name: &str, pass: u32) -> (HeightsReseat, Fixture) {
    let f = Fixture::new(name, pass);
    (HeightsReseat::new(&f), f)
}

fn heights_pull(name: &str, pass: u32) -> (HeightsPull, Fixture) {
    let f = Fixture::new(name, pass);
    (HeightsPull::new(&f), f)
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = mark_pull)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = mark_pull)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = mark_pull)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = mark_pull)]
fn instant_baseline((mut e, f): (MarkPull, Fixture)) -> (MarkPull, Fixture, u64) {
    let digest = e.all(black_box(&f.txs), black_box(&f.events));
    black_box((e, f, digest))
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = heights_reseat)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = heights_reseat)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = heights_reseat)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = heights_reseat)]
fn instant_reseat((mut e, f): (HeightsReseat, Fixture)) -> (HeightsReseat, Fixture, u64) {
    let digest = e.all(black_box(&f.txs), black_box(&f.events));
    black_box((e, f, digest))
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = heights_pull)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = heights_pull)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = heights_pull)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = heights_pull)]
fn instant_pull((mut e, f): (HeightsPull, Fixture)) -> (HeightsPull, Fixture, u64) {
    let digest = e.all(black_box(&f.txs), black_box(&f.events));
    black_box((e, f, digest))
}

fn mark_unforced(name: &str, pass: u32) -> (MarkUnforced, Fixture) {
    let f = Fixture::new(name, pass);
    (MarkUnforced::new(&f), f)
}

#[library_benchmark]
#[benches::settled(args = [("settled", 100), ("settled", 50), ("settled", 30), ("settled", 20), ("settled", 5)], setup = mark_unforced)]
#[benches::mixed(args = [("mixed", 100), ("mixed", 50), ("mixed", 30), ("mixed", 20), ("mixed", 5)], setup = mark_unforced)]
#[benches::churn(args = [("churn", 100), ("churn", 50), ("churn", 30), ("churn", 20), ("churn", 5)], setup = mark_unforced)]
#[benches::lazy(args = [("lazy", 100), ("lazy", 50), ("lazy", 30), ("lazy", 20), ("lazy", 5)], setup = mark_unforced)]
fn instant_unforced((mut e, f): (MarkUnforced, Fixture)) -> (MarkUnforced, Fixture, u64) {
    let digest = e.all(black_box(&f.txs), black_box(&f.events));
    black_box((e, f, digest))
}

library_benchmark_group!(
    name = transactions;
    benchmarks = baseline, heights, heap
);

library_benchmark_group!(
    name = upkeep;
    benchmarks = upkeep_baseline, upkeep_height
);

library_benchmark_group!(
    name = instant;
    benchmarks = instant_baseline, instant_reseat, instant_pull, instant_unforced
);

main!(library_benchmark_groups = transactions, upkeep, instant);
