//! Instruction counts for the demand-bounded-push probe: the wall-clock
//! bench's comparisons (its doc says what each variant is), with the `nav`
//! group at 100, 1,000 and 9,000 abandoned screens and the `app` group,
//! whose baseline setup collects after each of its navigations over ten
//! thousand live nodes, at 100 and 1,000 only.
//!
//! Each fixture is built in setup, outside what is counted, and warmed by
//! one transaction where the bench runs one. Each bench hands the fixture
//! back, so its drop runs in teardown, uncounted.

use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use bough_experiments::rfd_0005_demand_bounded_push::{Fixture, Push, Refresh};

fn base(kept: usize, n: usize) -> Fixture {
    Fixture::new(kept, n, Refresh::Collect).warmed(Push::All)
}

fn fresh(kept: usize, n: usize) -> Fixture {
    Fixture::new(kept, n, Refresh::None)
}

fn garbage(kept: usize, n: usize) -> Fixture {
    fresh(kept, n).warmed(Push::All)
}

fn checked(kept: usize, n: usize) -> Fixture {
    base(kept, n).warmed(Push::Skip)
}

fn marked(kept: usize, n: usize) -> Fixture {
    fresh(kept, n).refreshed(Refresh::Mark).warmed(Push::Skip)
}

fn pruned(kept: usize, n: usize) -> Fixture {
    fresh(kept, n).refreshed(Refresh::Census).warmed(Push::All)
}

fn one(kept: usize, n: usize) -> Fixture {
    base(kept, n).navigated()
}

fn drop_fixture<T>(out: (T, Fixture)) {
    black_box(out.0);
}

fn click(mut f: Fixture, push: Push) -> (i64, Fixture) {
    (black_box(f.click(black_box(7), push)), f)
}

fn refresh(mut f: Fixture, r: Refresh) -> (usize, Fixture) {
    (black_box(f.arena.refresh(r)), f)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = base, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = base, teardown = drop_fixture)]
fn baseline(f: Fixture) -> (i64, Fixture) {
    click(f, Push::All)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = garbage, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = garbage, teardown = drop_fixture)]
fn garbage_tx(f: Fixture) -> (i64, Fixture) {
    click(f, Push::All)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = checked, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = checked, teardown = drop_fixture)]
fn checked_tx(f: Fixture) -> (i64, Fixture) {
    click(f, Push::Skip)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = marked, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = marked, teardown = drop_fixture)]
fn skip(f: Fixture) -> (i64, Fixture) {
    click(f, Push::Skip)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = pruned, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = pruned, teardown = drop_fixture)]
fn pruned_tx(f: Fixture) -> (i64, Fixture) {
    click(f, Push::All)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = fresh, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = fresh, teardown = drop_fixture)]
fn mark(f: Fixture) -> (usize, Fixture) {
    refresh(f, Refresh::Mark)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = fresh, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = fresh, teardown = drop_fixture)]
fn census(f: Fixture) -> (usize, Fixture) {
    refresh(f, Refresh::Census)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = fresh, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = fresh, teardown = drop_fixture)]
fn collect(f: Fixture) -> (usize, Fixture) {
    refresh(f, Refresh::Collect)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = one, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = one, teardown = drop_fixture)]
fn census_one(f: Fixture) -> (usize, Fixture) {
    refresh(f, Refresh::Census)
}

#[library_benchmark]
#[benches::nav(args = [(0, 100), (0, 1_000), (0, 9_000)], setup = one, teardown = drop_fixture)]
#[benches::app(args = [(430, 100), (430, 1_000)], setup = one, teardown = drop_fixture)]
fn collect_one(f: Fixture) -> (usize, Fixture) {
    refresh(f, Refresh::Collect)
}

library_benchmark_group!(
    name = demand_bounded_push;
    benchmarks = baseline, garbage_tx, checked_tx, skip, pruned_tx, mark, census, collect,
        census_one, collect_one
);

main!(library_benchmark_groups = demand_bounded_push);
