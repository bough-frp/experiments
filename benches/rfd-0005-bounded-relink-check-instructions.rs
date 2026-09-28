//! Instruction counts for the bounded relink check, RFD 5: the same
//! comparisons as the wall-clock bench, on the same workloads.
//!
//! `moves_*` runs a workload's 300 transactions (the subgraphs built during
//! each, then its commit) through one checker; the graph, the checker's
//! initial order or summaries, and the workload are built in `setup`, and
//! everything is handed back so its drop isn't counted. Divide by the
//! workload's moves, from the counts, for a cost per move. `build_*` builds
//! every subgraph the `lazy` workload builds during its transactions, with
//! no moves, so the difference from `build_baseline`, divided by the nodes
//! built, is what keeping the order or the summaries adds to each node.

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use bough_experiments::rfd_0005_bounded_relink_check::{
    Baseline, Build, Checker, Pk, Run, Summaries, Tx, workload,
};

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

fn txs_summaries(name: &str) -> (Run<Summaries>, Vec<Tx>) {
    transactions(name)
}

fn builds_baseline(name: &str) -> (Run<Baseline>, Vec<Build>) {
    builds(name)
}

fn builds_pk(name: &str) -> (Run<Pk>, Vec<Build>) {
    builds(name)
}

fn builds_summaries(name: &str) -> (Run<Summaries>, Vec<Build>) {
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
#[bench::settled(args = ("settled"), setup = txs_summaries)]
#[bench::mixed(args = ("mixed"), setup = txs_summaries)]
#[bench::churn(args = ("churn"), setup = txs_summaries)]
#[bench::lazy(args = ("lazy"), setup = txs_summaries)]
fn moves_summaries(input: (Run<Summaries>, Vec<Tx>)) -> (Run<Summaries>, Vec<Tx>, usize) {
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
#[bench::lazy(args = ("lazy"), setup = builds_summaries)]
fn build_summaries(input: (Run<Summaries>, Vec<Build>)) -> (Run<Summaries>, Vec<Build>) {
    black_box(build(input))
}

library_benchmark_group!(
    name = moves;
    benchmarks = moves_baseline, moves_pk, moves_summaries
);

library_benchmark_group!(
    name = build;
    benchmarks = build_baseline, build_pk, build_summaries
);

main!(library_benchmark_groups = moves, build);
