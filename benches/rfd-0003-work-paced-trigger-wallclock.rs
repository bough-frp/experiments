//! Wall-clock times for the work-paced trigger probe: a long run of
//! navigations and clicks under each collection policy, collections
//! included, the longest pause each policy makes, and what a work term
//! costs a transaction when there is no garbage.
//!
//! Groups, each `<group>/<policy>/3900` but `fast-path/<policy>`:
//!
//! - `nav`, `app`: `WINDOW` units (1,300 navigations, 2,600 clicks) from
//!   the policy's first collection, on F66's navigation shape and on the
//!   `app` shape with 430 screens kept live beside it. The ratio to
//!   `baseline` (collect after every navigation) is the policy's total
//!   cost against that one's. Divide either by 3,900 for a unit.
//! - `pause-nav`, `pause-app`: the largest single collection the same
//!   window makes, the unit's longest pause.
//! - `fast-path`: one click and the trigger after it, on the `app` arena
//!   just collected, where the work terms find nothing. Here `baseline` is
//!   RFD 3's trigger, which an engine pays anyway, so each ratio is what a
//!   work term adds to a transaction.
//!
//! - `uneven-spread`, `uneven-sparse`, and `pause-uneven-spread`,
//!   `pause-uneven-sparse`: the same for the uneven workload (four inputs
//!   at uneven rates, live regions that grow, navigation garbage off every
//!   frequent input or off the tenth and hundredth only), each
//!   `<group>/<policy>/3600` over `UNEVEN_POLICIES`, `UNEVEN_WINDOW` units
//!   after `UNEVEN_WARMUP`. Divide by 3,600 for a unit.
//!
//! - `uneven-lagging`: `uneven-spread`'s run with every screen off the
//!   hundredth input, `uneven-lagging/<policy>/3600` under `baseline`,
//!   `excess` and `marked`, the fix for misses on a lagging input.
//!
//! Every run is cloned in `iter_batched`'s setup and handed back so that
//! neither the clone nor its drop is timed.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};

use bough_experiments::rfd_0003_work_paced_trigger::{
    APP_KEPT, MIXES, Mix, POLICIES, Policy, Run, UNEVEN_POLICIES, UNEVEN_WINDOW, Uneven, WINDOW,
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

fn work_paced_trigger(c: &mut Criterion) {
    for (shape, kept) in [("nav", 0), ("app", APP_KEPT)] {
        let mut g = c.benchmark_group(shape);
        g.sampling_mode(SamplingMode::Flat);
        g.sample_size(10);
        g.measurement_time(Duration::from_secs(8));
        for p in POLICIES {
            let r = Run::new(kept, p).warmed();
            batched(&mut g, p.name(), WINDOW, &r, |r| r.run(black_box(WINDOW)));
        }
        g.finish();

        let mut g = c.benchmark_group(format!("pause-{shape}"));
        g.sampling_mode(SamplingMode::Flat);
        g.measurement_time(Duration::from_millis(500));
        for p in POLICIES {
            let r = Run::before_largest(kept, p, WINDOW);
            batched(&mut g, p.name(), WINDOW, &r, Run::collect);
        }
        g.finish();
    }

    for mix in MIXES {
        let mut g = c.benchmark_group(format!("uneven-{}", mix.name()));
        g.sampling_mode(SamplingMode::Flat);
        g.sample_size(10);
        g.measurement_time(Duration::from_secs(8));
        for p in UNEVEN_POLICIES {
            let r = Uneven::new(mix, p, false).warmed();
            batched(&mut g, p.name(), UNEVEN_WINDOW, &r, |r| {
                r.run(black_box(UNEVEN_WINDOW))
            });
        }
        g.finish();

        let mut g = c.benchmark_group(format!("pause-uneven-{}", mix.name()));
        g.sampling_mode(SamplingMode::Flat);
        g.measurement_time(Duration::from_millis(500));
        for p in UNEVEN_POLICIES {
            let r = Uneven::before_largest(mix, p, UNEVEN_WINDOW);
            batched(&mut g, p.name(), UNEVEN_WINDOW, &r, Uneven::collect);
        }
        g.finish();
    }

    let mut g = c.benchmark_group("uneven-lagging");
    g.sampling_mode(SamplingMode::Flat);
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(8));
    for p in [Policy::Baseline, Policy::Excess, Policy::Marked] {
        let r = Uneven::new(Mix::Lagging, p, false).warmed();
        batched(&mut g, p.name(), UNEVEN_WINDOW, &r, |r| {
            r.run(black_box(UNEVEN_WINDOW))
        });
    }
    g.finish();

    let mut g = c.benchmark_group("fast-path");
    g.measurement_time(Duration::from_millis(300));
    for (id, p) in [
        ("baseline", Policy::Rfd3),
        ("excess", Policy::Excess),
        ("total", Policy::Total),
    ] {
        let mut r = Run::fast_path(APP_KEPT, p);
        // Clicks only, so no garbage is made. `total` still collects every
        // four hundred or so clicks, as it would in use, and that is timed.
        g.bench_function(id, |b| {
            b.iter(|| {
                let navigated = black_box(r.click());
                black_box(r.settle(navigated))
            })
        });
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(200));
    targets = work_paced_trigger
}
criterion_main!(benches);
