//! Instruction counts for the work-paced trigger probe: the wall-clock
//! bench's comparisons (its doc says what each is) at the same sizes, since
//! the longest run, `app` under `baseline`, is about 1.6 billion
//! instructions.
//!
//! - `run`: `WINDOW` units (1,300 navigations, 2,600 clicks) from the
//!   policy's first collection, collections included. Divide by 3,900 for
//!   the cost per unit.
//! - `pause`: the largest collection the same window makes.
//! - `fast_path_unit`: one click and the trigger after it on a collected
//!   arena, under RFD 3's trigger and under each work term. No collection
//!   runs in it; the wall-clock bench's loop also counts `total`'s.
//!
//! Every run is built and warmed in setup, outside what is counted, and
//! handed back so that its drop runs in teardown.

use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use bough_experiments::rfd_0003_work_paced_trigger::{APP_KEPT, Policy, Run, WINDOW};

fn warmed(kept: usize, p: Policy) -> Run {
    Run::new(kept, p).warmed()
}

fn before_largest(kept: usize, p: Policy) -> Run {
    Run::before_largest(kept, p, WINDOW)
}

fn fast_path(kept: usize, p: Policy) -> Run {
    Run::fast_path(kept, p)
}

fn drop_run<T>(out: (T, Run)) {
    black_box(out.0);
}

#[library_benchmark]
#[benches::nav(
    args = [(0, Policy::Baseline), (0, Policy::Rfd3), (0, Policy::Excess), (0, Policy::Total)],
    setup = warmed,
    teardown = drop_run
)]
#[benches::app(
    args = [
        (APP_KEPT, Policy::Baseline),
        (APP_KEPT, Policy::Rfd3),
        (APP_KEPT, Policy::Excess),
        (APP_KEPT, Policy::Total)
    ],
    setup = warmed,
    teardown = drop_run
)]
fn run(mut r: Run) -> (i64, Run) {
    (black_box(r.run(black_box(WINDOW))), r)
}

#[library_benchmark]
#[benches::nav(
    args = [(0, Policy::Baseline), (0, Policy::Rfd3), (0, Policy::Excess), (0, Policy::Total)],
    setup = before_largest,
    teardown = drop_run
)]
#[benches::app(
    args = [
        (APP_KEPT, Policy::Baseline),
        (APP_KEPT, Policy::Rfd3),
        (APP_KEPT, Policy::Excess),
        (APP_KEPT, Policy::Total)
    ],
    setup = before_largest,
    teardown = drop_run
)]
fn pause(mut r: Run) -> (usize, Run) {
    (black_box(r.collect()), r)
}

#[library_benchmark]
#[benches::app(
    args = [(APP_KEPT, Policy::Rfd3), (APP_KEPT, Policy::Excess), (APP_KEPT, Policy::Total)],
    setup = fast_path,
    teardown = drop_run
)]
fn fast_path_unit(mut r: Run) -> (Option<usize>, Run) {
    let navigated = r.click();
    (black_box(r.settle(navigated)), r)
}

library_benchmark_group!(
    name = work_paced_trigger;
    benchmarks = run, pause, fast_path_unit
);

main!(library_benchmark_groups = work_paced_trigger);
