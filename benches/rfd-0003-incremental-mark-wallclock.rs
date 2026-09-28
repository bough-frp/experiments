//! Wall-clock times for the incremental mark probe, on the `app` shape
//! (about 10,000 live nodes), against the atomic collection under the
//! work-paced trigger.
//!
//! Groups, each `<group>/<variant>/<param>` with `baseline` the atomic
//! collection on an arena with no barriers compiled in:
//!
//! - `incremental-run`: `WINDOW` units (1,300 navigations, 2,600 clicks)
//!   from the pace's first ended cycle, collection work included;
//!   `atomic-barriers` is the atomic collection on the barriered arena, and
//!   `k<k>` and `k<k>-atomic-sweep` the incremental paces (budget `k`
//!   mark-node equivalents a unit; the prune and sweep sliced, or atomic).
//!   The ratio is the pace's total cost against the atomic one's. Divide
//!   either by 3,900 for a unit.
//! - `incremental-pause`: the unit the counts' model rates costliest in the
//!   same window, its work and its collection slice: the worst pause.
//! - `incremental-fast-click`, `incremental-fast-navigate` (param 1): one
//!   unit's work with no collection work on a freshly collected arena with
//!   no garbage, on the barriered arena with no cycle (`idle`), mid-mark
//!   (`mark`) and mid-sweep (`sweep`, where transactions test colour).
//!   Each ratio is what the barriers cost that unit.
//!
//! Every run is cloned in `iter_batched`'s setup and handed back so that
//! neither the clone nor its drop is timed.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};

use bough_experiments::rfd_0003_incremental_mark::{APP_KEPT, PACES, Pace, Phase, Run, WINDOW};

fn batched<R: Clone, T>(
    g: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    id: &str,
    param: usize,
    r: &R,
    f: impl Fn(&mut R) -> T,
) {
    g.bench_function(BenchmarkId::new(id, param), |b| {
        b.iter_batched(
            || r.clone(),
            |mut r| (black_box(f(&mut r)), r),
            BatchSize::PerIteration,
        )
    });
}

/// The fixed-budget paces; the debt paces run for the counts only.
fn benched() -> impl Iterator<Item = Pace> {
    PACES
        .into_iter()
        .filter(|p| matches!(p, Pace::Fixed { .. }))
}

fn incremental_mark(c: &mut Criterion) {
    let mut g = c.benchmark_group("incremental-run");
    g.sampling_mode(SamplingMode::Flat);
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(6));
    let r = Run::<false>::new(APP_KEPT, Pace::Atomic).warmed();
    batched(&mut g, "baseline", WINDOW, &r, |r| r.run(black_box(WINDOW)));
    let r = Run::<true>::new(APP_KEPT, Pace::Atomic).warmed();
    batched(&mut g, "atomic-barriers", WINDOW, &r, |r| {
        r.run(black_box(WINDOW))
    });
    for p in benched() {
        let r = Run::<true>::new(APP_KEPT, p).warmed();
        batched(&mut g, &p.name(), WINDOW, &r, |r| r.run(black_box(WINDOW)));
    }
    g.finish();

    let mut g = c.benchmark_group("incremental-pause");
    g.sampling_mode(SamplingMode::Flat);
    g.measurement_time(Duration::from_millis(500));
    let r = Run::<false>::before_worst(APP_KEPT, Pace::Atomic, WINDOW);
    batched(&mut g, "baseline", WINDOW, &r, Run::unit);
    for p in benched() {
        let r = Run::<true>::before_worst(APP_KEPT, p, WINDOW);
        batched(&mut g, &p.name(), WINDOW, &r, Run::unit);
    }
    g.finish();

    for (group, navigate) in [
        ("incremental-fast-click", false),
        ("incremental-fast-navigate", true),
    ] {
        let mut g = c.benchmark_group(group);
        g.measurement_time(Duration::from_millis(500));
        let r = Run::<false>::fast_path(APP_KEPT, Phase::Idle, navigate);
        batched(&mut g, "baseline", 1, &r, Run::step);
        for (id, phase) in [
            ("idle", Phase::Idle),
            ("mark", Phase::Mark),
            ("sweep", Phase::Sweep),
        ] {
            let r = Run::<true>::fast_path(APP_KEPT, phase, navigate);
            batched(&mut g, id, 1, &r, Run::step);
        }
        g.finish();
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(200));
    targets = incremental_mark
}
criterion_main!(benches);
