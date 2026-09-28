//! Instruction counts for the incremental mark probe, on the `app` shape
//! (about 10,000 live nodes), against the atomic collection under the
//! work-paced trigger. The wall-clock bench makes the same comparisons.
//!
//! - `run`: `WINDOW` units (1,300 navigations, 2,600 clicks) from the
//!   pace's first ended cycle, collection work included. `atomic` is the
//!   baseline, an arena with no barriers compiled in; `atomic_barriers` is
//!   the same collection on the arena with them. Divide by 3,900 for the
//!   total cost of a unit.
//! - `pause`: the unit the counts' model rates costliest in the same
//!   window, its work and its collection slice: the pace's worst pause.
//! - `fast_path_click`, `fast_path_navigate`: one unit's work and no
//!   collection work, on a freshly collected arena with no garbage: the
//!   barrier-free arena (`atomic`), and the barriered one with no cycle
//!   running, mid-mark, and mid-sweep (where transactions test colour).
//!   What the barriers cost a unit is each against `atomic`.
//!
//! The debt paces aren't benched: the counts show their cycles running
//! hundreds to thousands of units while garbage builds, so their windows
//! measure garbage, not the collector.
//!
//! Every run is built and replayed in setup, outside what is counted, and
//! handed back so that its drop runs in teardown.

use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use bough_experiments::rfd_0003_incremental_mark::{APP_KEPT, Pace, Phase, Run, WINDOW};

fn warmed_atomic() -> Run<false> {
    Run::new(APP_KEPT, Pace::Atomic).warmed()
}

fn warmed(p: Pace) -> Run<true> {
    Run::new(APP_KEPT, p).warmed()
}

fn worst_atomic() -> Run<false> {
    Run::before_worst(APP_KEPT, Pace::Atomic, WINDOW)
}

fn worst(p: Pace) -> Run<true> {
    Run::before_worst(APP_KEPT, p, WINDOW)
}

fn fast_atomic(navigate: bool) -> Run<false> {
    Run::fast_path(APP_KEPT, Phase::Idle, navigate)
}

fn fast(phase: Phase, navigate: bool) -> Run<true> {
    Run::fast_path(APP_KEPT, phase, navigate)
}

fn drop_run<T, R>(out: (T, R)) {
    black_box(out.1);
}

#[library_benchmark]
#[bench::atomic(setup = warmed_atomic, teardown = drop_run)]
fn run_atomic(mut r: Run<false>) -> (i64, Run<false>) {
    (black_box(r.run(black_box(WINDOW))), r)
}

#[library_benchmark]
#[benches::paces(
    args = [
        Pace::Atomic,
        Pace::Fixed { k: 250, sliced: true },
        Pace::Fixed { k: 1_000, sliced: true },
        Pace::Fixed { k: 4_000, sliced: true },
        Pace::Fixed { k: 250, sliced: false },
        Pace::Fixed { k: 1_000, sliced: false },
        Pace::Fixed { k: 4_000, sliced: false }
    ],
    setup = warmed,
    teardown = drop_run
)]
fn run(mut r: Run<true>) -> (i64, Run<true>) {
    (black_box(r.run(black_box(WINDOW))), r)
}

#[library_benchmark]
#[bench::atomic(setup = worst_atomic, teardown = drop_run)]
fn pause_atomic(mut r: Run<false>) -> (usize, Run<false>) {
    (black_box(r.unit().slice.marked), r)
}

#[library_benchmark]
#[benches::paces(
    args = [
        Pace::Fixed { k: 250, sliced: true },
        Pace::Fixed { k: 1_000, sliced: true },
        Pace::Fixed { k: 4_000, sliced: true },
        Pace::Fixed { k: 250, sliced: false },
        Pace::Fixed { k: 1_000, sliced: false },
        Pace::Fixed { k: 4_000, sliced: false }
    ],
    setup = worst,
    teardown = drop_run
)]
fn pause(mut r: Run<true>) -> (usize, Run<true>) {
    (black_box(r.unit().slice.marked), r)
}

#[library_benchmark]
#[bench::click(args = (false), setup = fast_atomic, teardown = drop_run)]
#[bench::navigate(args = (true), setup = fast_atomic, teardown = drop_run)]
fn fast_path_atomic(mut r: Run<false>) -> ((usize, usize, usize), Run<false>) {
    (black_box(r.step()), r)
}

#[library_benchmark]
#[benches::click(
    args = [(Phase::Idle, false), (Phase::Mark, false), (Phase::Sweep, false)],
    setup = fast,
    teardown = drop_run
)]
#[benches::navigate(
    args = [(Phase::Idle, true), (Phase::Mark, true), (Phase::Sweep, true)],
    setup = fast,
    teardown = drop_run
)]
fn fast_path(mut r: Run<true>) -> ((usize, usize, usize), Run<true>) {
    (black_box(r.step()), r)
}

library_benchmark_group!(
    name = incremental_mark;
    benchmarks = run_atomic, run, pause_atomic, pause, fast_path_atomic, fast_path
);

main!(library_benchmark_groups = incremental_mark);
