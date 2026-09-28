//! Instruction counts for heap versus mark on quiet regions, RFD 5: the
//! same comparisons as the wall-clock bench, on the same workloads.
//!
//! Each bench runs one cycle of the fixture's events, `CYCLE` (64)
//! transactions, through one scheduler: `baseline` (RFD 5's mark and flat
//! loop), `heap` (a binary heap by height, fire-only) and `bucket` (a
//! bucket queue by height, fire-only). Divide by 64 for a cost per
//! transaction. `ui_small` and `ui_large` take the filters' pass rate in
//! percent; the quiet fraction each gives is in the counts (`cargo test
//! --release --lib rfd_0005_heap_vs_mark_on_quiet_regions::tests::counts
//! -- --nocapture`). `frame` takes the frame's width, and all of it fires.
//!
//! The fixture is built in `setup` and every scheduler runs a cycle on it
//! there, so the scratch vectors and the heap have grown, what is counted
//! is the steady state, and the counted cycle starts at the first event,
//! as the counts' does. The fixture is handed back so its drop isn't
//! counted.

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use bough_experiments::rfd_0005_heap_vs_mark_on_quiet_regions::{CYCLE, Fixture, Sched};

const TXS: usize = CYCLE;

fn warm(mut f: Fixture) -> Fixture {
    for s in Sched::ALL {
        f.steps(s, TXS);
    }
    f
}

fn ui_small(pass: u32) -> Fixture {
    warm(Fixture::ui_small(pass))
}

fn ui_large(pass: u32) -> Fixture {
    warm(Fixture::ui_large(pass))
}

fn frame(w: u32) -> Fixture {
    warm(Fixture::frame(w))
}

fn run(mut f: Fixture, s: Sched) -> (u64, Fixture) {
    (black_box(f.steps(s, TXS)), f)
}

fn drop_fixture(out: (u64, Fixture)) {
    black_box(out.0);
}

#[library_benchmark]
#[benches::ui_small(args = [100, 90, 75, 50, 25, 10, 5, 2, 1, 0], setup = ui_small, teardown = drop_fixture)]
#[benches::ui_large(args = [100, 90, 75, 50, 25, 10, 5, 2, 1, 0], setup = ui_large, teardown = drop_fixture)]
#[benches::frame(args = [64, 1024], setup = frame, teardown = drop_fixture)]
fn baseline(f: Fixture) -> (u64, Fixture) {
    run(f, Sched::Mark)
}

#[library_benchmark]
#[benches::ui_small(args = [100, 90, 75, 50, 25, 10, 5, 2, 1, 0], setup = ui_small, teardown = drop_fixture)]
#[benches::ui_large(args = [100, 90, 75, 50, 25, 10, 5, 2, 1, 0], setup = ui_large, teardown = drop_fixture)]
#[benches::frame(args = [64, 1024], setup = frame, teardown = drop_fixture)]
fn heap(f: Fixture) -> (u64, Fixture) {
    run(f, Sched::Heap)
}

#[library_benchmark]
#[benches::ui_small(args = [100, 90, 75, 50, 25, 10, 5, 2, 1, 0], setup = ui_small, teardown = drop_fixture)]
#[benches::ui_large(args = [100, 90, 75, 50, 25, 10, 5, 2, 1, 0], setup = ui_large, teardown = drop_fixture)]
#[benches::frame(args = [64, 1024], setup = frame, teardown = drop_fixture)]
fn bucket(f: Fixture) -> (u64, Fixture) {
    run(f, Sched::Bucket)
}

library_benchmark_group!(
    name = heap_vs_mark;
    benchmarks = baseline, heap, bucket
);

main!(library_benchmark_groups = heap_vs_mark);
