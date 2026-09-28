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
//! - `incremental-ext-run`, `incremental-ext-pause`: the follow-ups on
//!   `app`, as `incremental-run` and `incremental-pause`: `k1000` again, and
//!   the variants the extended counts compare: the trigger reset when the
//!   mark ends (`-early`) or counting floating garbage (`-born`), `k2000`,
//!   and the debt on the whole region (`region-debt<c>`).
//! - `incremental-uneven-run`, `incremental-uneven-pause`: the work-paced
//!   probe's uneven workload with guards dropped, `UNEVEN_UNITS` units after
//!   its warm-up (divide by 3,600 for a unit), and its costliest unit;
//!   `baseline` the atomic collection with no barriers compiled in.
//!
//! Every run is cloned in `iter_batched`'s setup and handed back so that
//! neither the clone nor its drop is timed.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};

use bough_experiments::rfd_0003_incremental_mark::{
    APP_KEPT, PACES, Pace, Phase, Reset, Run, UNEVEN_UNITS, Uneven, WINDOW,
};

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

/// The follow-ups' variants on `app`, each a pace and a trigger reset.
fn extended() -> [(Pace, Reset); 8] {
    let fixed = |k| Pace::Fixed { k, sliced: true };
    [
        (fixed(1_000), Reset::End),
        (fixed(250), Reset::Born),
        (fixed(500), Reset::Born),
        (fixed(1_000), Reset::Early),
        (fixed(1_000), Reset::Born),
        (fixed(2_000), Reset::End),
        (Pace::RegionDebt { c: 4 }, Reset::End),
        (Pace::RegionDebt { c: 8 }, Reset::End),
    ]
}

fn incremental_extended(c: &mut Criterion) {
    let mut g = c.benchmark_group("incremental-ext-run");
    g.sampling_mode(SamplingMode::Flat);
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(6));
    let r = Run::<false>::new(APP_KEPT, Pace::Atomic).warmed();
    batched(&mut g, "baseline", WINDOW, &r, |r| r.run(black_box(WINDOW)));
    for (p, reset) in extended() {
        let r = Run::<true>::with_reset(APP_KEPT, p, reset).warmed();
        batched(&mut g, &r.name(), WINDOW, &r, |r| r.run(black_box(WINDOW)));
    }
    g.finish();

    let mut g = c.benchmark_group("incremental-ext-pause");
    g.sampling_mode(SamplingMode::Flat);
    g.measurement_time(Duration::from_millis(500));
    let r = Run::<false>::before_worst(APP_KEPT, Pace::Atomic, WINDOW);
    batched(&mut g, "baseline", WINDOW, &r, Run::unit);
    for (p, reset) in extended() {
        let r = Run::<true>::before_worst_reset(APP_KEPT, p, reset, WINDOW);
        batched(&mut g, &r.name(), WINDOW, &r, Run::unit);
    }
    g.finish();

    let k1000 = Pace::Fixed {
        k: 1_000,
        sliced: true,
    };
    let mut g = c.benchmark_group("incremental-uneven-run");
    g.sampling_mode(SamplingMode::Flat);
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(10));
    let r = Uneven::<false>::new(Pace::Atomic).warmed();
    batched(&mut g, "baseline", UNEVEN_UNITS, &r, |r| {
        r.run(black_box(UNEVEN_UNITS))
    });
    let r = Uneven::<true>::new(Pace::Atomic).warmed();
    batched(&mut g, "atomic-barriers", UNEVEN_UNITS, &r, |r| {
        r.run(black_box(UNEVEN_UNITS))
    });
    let r = Uneven::<true>::new(k1000).warmed();
    batched(&mut g, "k1000", UNEVEN_UNITS, &r, |r| {
        r.run(black_box(UNEVEN_UNITS))
    });
    g.finish();

    let mut g = c.benchmark_group("incremental-uneven-pause");
    g.sampling_mode(SamplingMode::Flat);
    g.measurement_time(Duration::from_millis(500));
    let r = Uneven::<false>::before_worst(Pace::Atomic, UNEVEN_UNITS);
    batched(&mut g, "baseline", UNEVEN_UNITS, &r, Uneven::unit);
    let r = Uneven::<true>::before_worst(k1000, UNEVEN_UNITS);
    batched(&mut g, "k1000", UNEVEN_UNITS, &r, Uneven::unit);
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(200));
    targets = incremental_mark, incremental_extended
}
criterion_main!(benches);
