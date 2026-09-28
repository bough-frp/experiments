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
//! - `uneven_run`, `uneven_pause` (group `uneven`): the same two for the
//!   uneven workload, four inputs at uneven rates with growing live regions,
//!   `UNEVEN_WINDOW` units after `UNEVEN_WARMUP`, under each garbage mix and
//!   each of `UNEVEN_POLICIES`. Divide `uneven_run` by 3,600 for the cost
//!   per unit. Run it alone with the filter `*::uneven::*`.
//! - `lagging_run` (group `spurious_missed`): `uneven_run` with every screen
//!   off the hundredth input (`Mix::Lagging`), under `baseline`, `excess`
//!   and `marked`, whose reference pass it counts. Divide by 3,600. Run it
//!   alone with the filter `*::spurious_missed::*`.
//!
//! Every run is built and warmed in setup, outside what is counted, and
//! handed back so that its drop runs in teardown.

use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use bough_experiments::rfd_0003_work_paced_trigger::{
    APP_KEPT, Mix, Policy, Run, UNEVEN_WINDOW, Uneven, WINDOW,
};

fn warmed(kept: usize, p: Policy) -> Run {
    Run::new(kept, p).warmed()
}

fn before_largest(kept: usize, p: Policy) -> Run {
    Run::before_largest(kept, p, WINDOW)
}

fn fast_path(kept: usize, p: Policy) -> Run {
    Run::fast_path(kept, p)
}

fn drop_run<T, R>(out: (T, R)) {
    black_box(out.0);
}

fn uneven_warmed(mix: Mix, p: Policy) -> Uneven {
    Uneven::new(mix, p, false).warmed()
}

fn uneven_before_largest(mix: Mix, p: Policy) -> Uneven {
    Uneven::before_largest(mix, p, UNEVEN_WINDOW)
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

#[library_benchmark]
#[benches::spread(
    args = [
        (Mix::Spread, Policy::Baseline),
        (Mix::Spread, Policy::Rfd3),
        (Mix::Spread, Policy::Excess),
        (Mix::Spread, Policy::Total),
        (Mix::Spread, Policy::Decay),
        (Mix::Spread, Policy::Backoff)
    ],
    setup = uneven_warmed,
    teardown = drop_run
)]
#[benches::sparse(
    args = [
        (Mix::Sparse, Policy::Baseline),
        (Mix::Sparse, Policy::Rfd3),
        (Mix::Sparse, Policy::Excess),
        (Mix::Sparse, Policy::Total),
        (Mix::Sparse, Policy::Decay),
        (Mix::Sparse, Policy::Backoff)
    ],
    setup = uneven_warmed,
    teardown = drop_run
)]
fn uneven_run(mut r: Uneven) -> (i64, Uneven) {
    (black_box(r.run(black_box(UNEVEN_WINDOW))), r)
}

#[library_benchmark]
#[benches::lagging(
    args = [
        (Mix::Lagging, Policy::Baseline),
        (Mix::Lagging, Policy::Excess),
        (Mix::Lagging, Policy::Marked)
    ],
    setup = uneven_warmed,
    teardown = drop_run
)]
fn lagging_run(mut r: Uneven) -> (i64, Uneven) {
    (black_box(r.run(black_box(UNEVEN_WINDOW))), r)
}

#[library_benchmark]
#[benches::spread(
    args = [
        (Mix::Spread, Policy::Baseline),
        (Mix::Spread, Policy::Rfd3),
        (Mix::Spread, Policy::Excess),
        (Mix::Spread, Policy::Total),
        (Mix::Spread, Policy::Decay),
        (Mix::Spread, Policy::Backoff)
    ],
    setup = uneven_before_largest,
    teardown = drop_run
)]
#[benches::sparse(
    args = [
        (Mix::Sparse, Policy::Baseline),
        (Mix::Sparse, Policy::Rfd3),
        (Mix::Sparse, Policy::Excess),
        (Mix::Sparse, Policy::Total),
        (Mix::Sparse, Policy::Decay),
        (Mix::Sparse, Policy::Backoff)
    ],
    setup = uneven_before_largest,
    teardown = drop_run
)]
fn uneven_pause(mut r: Uneven) -> (usize, Uneven) {
    (black_box(r.collect()), r)
}

library_benchmark_group!(
    name = work_paced_trigger;
    benchmarks = run, pause, fast_path_unit
);

library_benchmark_group!(
    name = uneven;
    benchmarks = uneven_run, uneven_pause
);

library_benchmark_group!(
    name = spurious_missed;
    benchmarks = lagging_run
);

main!(
    library_benchmark_groups = work_paced_trigger,
    uneven,
    spurious_missed
);
