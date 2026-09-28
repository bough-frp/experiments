//! Instruction counts for the small-side order, RFD 5: the same comparisons
//! as the wall-clock bench, on the earlier probe's workloads.
//!
//! `moves_*` runs a workload's 300 transactions (the subgraphs built during
//! each, then its commit) through one checker: `baseline` is the spike's
//! upstream walk and `pk` the Pearce–Kelly array, both from
//! `rfd_0005_bounded_relink_check`; `back` and `twoway` are this probe's
//! order-maintenance lists. The graph, the checker's initial order and the
//! workload are built in `setup`, and everything is handed back so its drop
//! isn't counted. Divide by the workload's moves, from the counts, for a
//! cost per move. `build_*` builds every subgraph the `lazy` workload builds
//! during its transactions, with no moves, so the difference from
//! `build_baseline`, divided by the nodes built, is what keeping the order
//! adds to each node. `back` and `twoway` build the same way, so `om`
//! stands for both.

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use bough_experiments::rfd_0005_bounded_relink_check::{
    Baseline, Build, Checker, Pk, Run, Tx, workload,
};
use bough_experiments::rfd_0005_small_side_order::{Back, TwoWay};

fn transactions<C: Checker>(name: &str) -> (Run<C>, Vec<Tx>) {
    let w = workload(name);
    (Run::new(&w), w.txs)
}

fn builds<C: Checker>(name: &str) -> (Run<C>, Vec<Build>) {
    let w = workload(name);
    (Run::new(&w), w.builds())
}

fn txs_baseline(name: &str) -> (Run<Baseline>, Vec<Tx>) {
    transactions(name)
}

fn txs_pk(name: &str) -> (Run<Pk>, Vec<Tx>) {
    transactions(name)
}

fn txs_back(name: &str) -> (Run<Back>, Vec<Tx>) {
    transactions(name)
}

fn txs_twoway(name: &str) -> (Run<TwoWay>, Vec<Tx>) {
    transactions(name)
}

fn builds_baseline(name: &str) -> (Run<Baseline>, Vec<Build>) {
    builds(name)
}

fn builds_pk(name: &str) -> (Run<Pk>, Vec<Build>) {
    builds(name)
}

fn builds_om(name: &str) -> (Run<Back>, Vec<Build>) {
    builds(name)
}

fn run<C: Checker>((mut run, txs): (Run<C>, Vec<Tx>)) -> (Run<C>, Vec<Tx>, usize) {
    let refused = run.all(black_box(&txs));
    (run, txs, refused)
}

fn build<C: Checker>((mut run, builds): (Run<C>, Vec<Build>)) -> (Run<C>, Vec<Build>) {
    for b in black_box(&builds) {
        run.build(b);
    }
    (run, builds)
}

#[library_benchmark]
#[bench::settled(args = ("settled"), setup = txs_baseline)]
#[bench::mixed(args = ("mixed"), setup = txs_baseline)]
#[bench::churn(args = ("churn"), setup = txs_baseline)]
#[bench::lazy(args = ("lazy"), setup = txs_baseline)]
fn moves_baseline(input: (Run<Baseline>, Vec<Tx>)) -> (Run<Baseline>, Vec<Tx>, usize) {
    black_box(run(input))
}

#[library_benchmark]
#[bench::settled(args = ("settled"), setup = txs_pk)]
#[bench::mixed(args = ("mixed"), setup = txs_pk)]
#[bench::churn(args = ("churn"), setup = txs_pk)]
#[bench::lazy(args = ("lazy"), setup = txs_pk)]
fn moves_pk(input: (Run<Pk>, Vec<Tx>)) -> (Run<Pk>, Vec<Tx>, usize) {
    black_box(run(input))
}

#[library_benchmark]
#[bench::settled(args = ("settled"), setup = txs_back)]
#[bench::mixed(args = ("mixed"), setup = txs_back)]
#[bench::churn(args = ("churn"), setup = txs_back)]
#[bench::lazy(args = ("lazy"), setup = txs_back)]
fn moves_back(input: (Run<Back>, Vec<Tx>)) -> (Run<Back>, Vec<Tx>, usize) {
    black_box(run(input))
}

#[library_benchmark]
#[bench::settled(args = ("settled"), setup = txs_twoway)]
#[bench::mixed(args = ("mixed"), setup = txs_twoway)]
#[bench::churn(args = ("churn"), setup = txs_twoway)]
#[bench::lazy(args = ("lazy"), setup = txs_twoway)]
fn moves_twoway(input: (Run<TwoWay>, Vec<Tx>)) -> (Run<TwoWay>, Vec<Tx>, usize) {
    black_box(run(input))
}

#[library_benchmark]
#[bench::lazy(args = ("lazy"), setup = builds_baseline)]
fn build_baseline(input: (Run<Baseline>, Vec<Build>)) -> (Run<Baseline>, Vec<Build>) {
    black_box(build(input))
}

#[library_benchmark]
#[bench::lazy(args = ("lazy"), setup = builds_pk)]
fn build_pk(input: (Run<Pk>, Vec<Build>)) -> (Run<Pk>, Vec<Build>) {
    black_box(build(input))
}

#[library_benchmark]
#[bench::lazy(args = ("lazy"), setup = builds_om)]
fn build_om(input: (Run<Back>, Vec<Build>)) -> (Run<Back>, Vec<Build>) {
    black_box(build(input))
}

library_benchmark_group!(
    name = moves;
    benchmarks = moves_baseline, moves_pk, moves_back, moves_twoway
);

library_benchmark_group!(
    name = build;
    benchmarks = build_baseline, build_pk, build_om
);

main!(library_benchmark_groups = moves, build);
