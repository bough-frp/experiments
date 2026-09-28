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
//!
//! `adversarial::adv_*` and `adversarial::cyc_*` run the adversary: one
//! switch moved 16 times, each to a new inner whose `up` nodes upstream sit
//! after the switch in the order while the switch's `down` nodes downstream
//! sit before it (`u<up>_d<down>`), all accepted, or with each new upstream
//! hung off the switch's downstream so every move is refused (`cyc_*`). The
//! graph, every new inner and the checker are built in `setup`; only the
//! moves are measured, so divide by 16 for a cost per move. `adv_baseline`
//! and `cyc_baseline` are the walk.
//!
//! `spacing::*_fresh` runs the lists with fresh spacing (a moved set takes
//! half its gap, away from where the next set to that spot goes) on the
//! workloads (`moves_*`) and the acyclic adversary (`adv_*`), to set against
//! `moves_back`, `moves_twoway`, `adv_back` and `adv_twoway`; the cyclic
//! adversary never reorders, so it is left out. `spacing_mixed::mix_*` runs
//! the mixed adversary: 16 moves, each to a new inner that reads a shared
//! upstream of `s` nodes before the switch in the order and `n` nodes of
//! its own after it, with 1,000 nodes downstream of the switch
//! (`s<s>_n<n>`); `mix_baseline` is the walk. Setup as for the adversary.
//!
//! `nosort::*_back_nosort` runs `back` with fresh spacing and a depth-first
//! search whose post-order moves without a sort, on the workloads
//! (`moves_*`), the adversary (`adv_*`, `cyc_*`) and the mixed adversary
//! (`mix_*`), to set against `moves_back_fresh`, `adv_back_fresh`,
//! `mix_back_fresh`, `cyc_back` and each group's baseline.

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use bough_experiments::rfd_0005_bounded_relink_check::{
    Baseline, Build, Checker, Pk, Run, Tx, workload,
};
use bough_experiments::rfd_0005_small_side_order::{
    Adversary, Back, BackFresh, BackNoSort, MIXED_DOWN, TwoWay, TwoWayFresh,
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

fn txs_back(name: &str) -> (Run<Back>, Vec<Tx>) {
    transactions(name)
}

fn txs_twoway(name: &str) -> (Run<TwoWay>, Vec<Tx>) {
    transactions(name)
}

fn txs_back_fresh(name: &str) -> (Run<BackFresh>, Vec<Tx>) {
    transactions(name)
}

fn txs_twoway_fresh(name: &str) -> (Run<TwoWayFresh>, Vec<Tx>) {
    transactions(name)
}

fn txs_back_nosort(name: &str) -> (Run<BackNoSort>, Vec<Tx>) {
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
#[bench::settled(args = ("settled"), setup = txs_back_fresh)]
#[bench::mixed(args = ("mixed"), setup = txs_back_fresh)]
#[bench::churn(args = ("churn"), setup = txs_back_fresh)]
#[bench::lazy(args = ("lazy"), setup = txs_back_fresh)]
fn moves_back_fresh(input: (Run<BackFresh>, Vec<Tx>)) -> (Run<BackFresh>, Vec<Tx>, usize) {
    black_box(run(input))
}

#[library_benchmark]
#[bench::settled(args = ("settled"), setup = txs_twoway_fresh)]
#[bench::mixed(args = ("mixed"), setup = txs_twoway_fresh)]
#[bench::churn(args = ("churn"), setup = txs_twoway_fresh)]
#[bench::lazy(args = ("lazy"), setup = txs_twoway_fresh)]
fn moves_twoway_fresh(input: (Run<TwoWayFresh>, Vec<Tx>)) -> (Run<TwoWayFresh>, Vec<Tx>, usize) {
    black_box(run(input))
}

#[library_benchmark]
#[bench::settled(args = ("settled"), setup = txs_back_nosort)]
#[bench::mixed(args = ("mixed"), setup = txs_back_nosort)]
#[bench::churn(args = ("churn"), setup = txs_back_nosort)]
#[bench::lazy(args = ("lazy"), setup = txs_back_nosort)]
fn moves_back_nosort(input: (Run<BackNoSort>, Vec<Tx>)) -> (Run<BackNoSort>, Vec<Tx>, usize) {
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

fn adversary<C: Checker>(up: usize, down: usize, cyclic: bool) -> Adversary<C> {
    Adversary::new(up, down, cyclic)
}

fn adv_setup_baseline(up: usize, down: usize) -> Adversary<Baseline> {
    adversary(up, down, false)
}

fn adv_setup_pk(up: usize, down: usize) -> Adversary<Pk> {
    adversary(up, down, false)
}

fn adv_setup_back(up: usize, down: usize) -> Adversary<Back> {
    adversary(up, down, false)
}

fn adv_setup_twoway(up: usize, down: usize) -> Adversary<TwoWay> {
    adversary(up, down, false)
}

fn adv_setup_back_fresh(up: usize, down: usize) -> Adversary<BackFresh> {
    adversary(up, down, false)
}

fn adv_setup_twoway_fresh(up: usize, down: usize) -> Adversary<TwoWayFresh> {
    adversary(up, down, false)
}

fn adv_setup_back_nosort(up: usize, down: usize) -> Adversary<BackNoSort> {
    adversary(up, down, false)
}

fn mix_setup_baseline(shared: usize, new: usize) -> Adversary<Baseline> {
    Adversary::mixed(shared, new, MIXED_DOWN)
}

fn mix_setup_back(shared: usize, new: usize) -> Adversary<Back> {
    Adversary::mixed(shared, new, MIXED_DOWN)
}

fn mix_setup_twoway(shared: usize, new: usize) -> Adversary<TwoWay> {
    Adversary::mixed(shared, new, MIXED_DOWN)
}

fn mix_setup_back_fresh(shared: usize, new: usize) -> Adversary<BackFresh> {
    Adversary::mixed(shared, new, MIXED_DOWN)
}

fn mix_setup_twoway_fresh(shared: usize, new: usize) -> Adversary<TwoWayFresh> {
    Adversary::mixed(shared, new, MIXED_DOWN)
}

fn mix_setup_back_nosort(shared: usize, new: usize) -> Adversary<BackNoSort> {
    Adversary::mixed(shared, new, MIXED_DOWN)
}

fn cyc_setup_baseline(up: usize, down: usize) -> Adversary<Baseline> {
    adversary(up, down, true)
}

fn cyc_setup_pk(up: usize, down: usize) -> Adversary<Pk> {
    adversary(up, down, true)
}

fn cyc_setup_back(up: usize, down: usize) -> Adversary<Back> {
    adversary(up, down, true)
}

fn cyc_setup_twoway(up: usize, down: usize) -> Adversary<TwoWay> {
    adversary(up, down, true)
}

fn cyc_setup_back_nosort(up: usize, down: usize) -> Adversary<BackNoSort> {
    adversary(up, down, true)
}

/// One benchmark function over every pair of sizes in `SIDES`.
macro_rules! grid {
    ($name:ident, $setup:ident, $checker:ty) => {
        #[library_benchmark]
        #[bench::u10_d10(args = (10, 10), setup = $setup)]
        #[bench::u10_d100(args = (10, 100), setup = $setup)]
        #[bench::u10_d1000(args = (10, 1_000), setup = $setup)]
        #[bench::u10_d10000(args = (10, 10_000), setup = $setup)]
        #[bench::u100_d10(args = (100, 10), setup = $setup)]
        #[bench::u100_d100(args = (100, 100), setup = $setup)]
        #[bench::u100_d1000(args = (100, 1_000), setup = $setup)]
        #[bench::u100_d10000(args = (100, 10_000), setup = $setup)]
        #[bench::u1000_d10(args = (1_000, 10), setup = $setup)]
        #[bench::u1000_d100(args = (1_000, 100), setup = $setup)]
        #[bench::u1000_d1000(args = (1_000, 1_000), setup = $setup)]
        #[bench::u1000_d10000(args = (1_000, 10_000), setup = $setup)]
        #[bench::u10000_d10(args = (10_000, 10), setup = $setup)]
        #[bench::u10000_d100(args = (10_000, 100), setup = $setup)]
        #[bench::u10000_d1000(args = (10_000, 1_000), setup = $setup)]
        #[bench::u10000_d10000(args = (10_000, 10_000), setup = $setup)]
        fn $name(mut input: Adversary<$checker>) -> (Adversary<$checker>, usize) {
            let refused = input.all();
            black_box((input, refused))
        }
    };
}

grid!(adv_baseline, adv_setup_baseline, Baseline);
grid!(adv_pk, adv_setup_pk, Pk);
grid!(adv_back, adv_setup_back, Back);
grid!(adv_twoway, adv_setup_twoway, TwoWay);
grid!(adv_back_fresh, adv_setup_back_fresh, BackFresh);
grid!(adv_twoway_fresh, adv_setup_twoway_fresh, TwoWayFresh);
grid!(cyc_baseline, cyc_setup_baseline, Baseline);
grid!(cyc_pk, cyc_setup_pk, Pk);
grid!(cyc_back, cyc_setup_back, Back);
grid!(cyc_twoway, cyc_setup_twoway, TwoWay);
grid!(adv_back_nosort, adv_setup_back_nosort, BackNoSort);
grid!(cyc_back_nosort, cyc_setup_back_nosort, BackNoSort);

/// One benchmark function over every pair of sizes in `SHARED` and `NEW`.
macro_rules! mixed_grid {
    ($name:ident, $setup:ident, $checker:ty) => {
        #[library_benchmark]
        #[bench::s10_n10(args = (10, 10), setup = $setup)]
        #[bench::s10_n100(args = (10, 100), setup = $setup)]
        #[bench::s10_n1000(args = (10, 1_000), setup = $setup)]
        #[bench::s10_n10000(args = (10, 10_000), setup = $setup)]
        #[bench::s100_n10(args = (100, 10), setup = $setup)]
        #[bench::s100_n100(args = (100, 100), setup = $setup)]
        #[bench::s100_n1000(args = (100, 1_000), setup = $setup)]
        #[bench::s100_n10000(args = (100, 10_000), setup = $setup)]
        #[bench::s1000_n10(args = (1_000, 10), setup = $setup)]
        #[bench::s1000_n100(args = (1_000, 100), setup = $setup)]
        #[bench::s1000_n1000(args = (1_000, 1_000), setup = $setup)]
        #[bench::s1000_n10000(args = (1_000, 10_000), setup = $setup)]
        #[bench::s10000_n10(args = (10_000, 10), setup = $setup)]
        #[bench::s10000_n100(args = (10_000, 100), setup = $setup)]
        #[bench::s10000_n1000(args = (10_000, 1_000), setup = $setup)]
        #[bench::s10000_n10000(args = (10_000, 10_000), setup = $setup)]
        #[bench::s100000_n10(args = (100_000, 10), setup = $setup)]
        #[bench::s100000_n100(args = (100_000, 100), setup = $setup)]
        #[bench::s100000_n1000(args = (100_000, 1_000), setup = $setup)]
        #[bench::s100000_n10000(args = (100_000, 10_000), setup = $setup)]
        fn $name(mut input: Adversary<$checker>) -> (Adversary<$checker>, usize) {
            let refused = input.all();
            black_box((input, refused))
        }
    };
}

mixed_grid!(mix_baseline, mix_setup_baseline, Baseline);
mixed_grid!(mix_back, mix_setup_back, Back);
mixed_grid!(mix_twoway, mix_setup_twoway, TwoWay);
mixed_grid!(mix_back_fresh, mix_setup_back_fresh, BackFresh);
mixed_grid!(mix_twoway_fresh, mix_setup_twoway_fresh, TwoWayFresh);
mixed_grid!(mix_back_nosort, mix_setup_back_nosort, BackNoSort);

library_benchmark_group!(
    name = moves;
    benchmarks = moves_baseline, moves_pk, moves_back, moves_twoway
);

library_benchmark_group!(
    name = build;
    benchmarks = build_baseline, build_pk, build_om
);

library_benchmark_group!(
    name = adversarial;
    benchmarks = adv_baseline, adv_pk, adv_back, adv_twoway,
        cyc_baseline, cyc_pk, cyc_back, cyc_twoway
);

library_benchmark_group!(
    name = spacing;
    benchmarks = moves_back_fresh, moves_twoway_fresh, adv_back_fresh, adv_twoway_fresh
);

library_benchmark_group!(
    name = spacing_mixed;
    benchmarks = mix_baseline, mix_back, mix_twoway, mix_back_fresh, mix_twoway_fresh
);

library_benchmark_group!(
    name = nosort;
    benchmarks = moves_back_nosort, adv_back_nosort, cyc_back_nosort, mix_back_nosort
);

main!(
    library_benchmark_groups = moves,
    build,
    adversarial,
    spacing,
    spacing_mixed,
    nosort
);
