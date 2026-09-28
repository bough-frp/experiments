//! Wall-clock times for the demand-bounded-push probe: one transaction on
//! the clicks of F66's navigation graph after 100, 1,000 and 9,000
//! abandoned screens, under each way of keeping the dead out of it, and
//! what each of those ways costs to refresh.
//!
//! Two groups. `nav` is F66's shape: the live set is the core and the
//! screen on show, about thirty nodes. `app` adds 430 screens kept live on
//! the clock, about ten thousand nodes the clicks don't reach, so a
//! liveness pass is no longer nearly free. Under RFD 3's trigger (collect
//! once allocations exceed what the last collection left) garbage never
//! outgrows the live set, so `nav` never sees more than a screen or two of
//! it in practice and F66's 9,000 screens are the manual policy's; `app`
//! at 100 abandoned screens (about 2,400 nodes) is inside what the trigger
//! allows, and at 1,000 and 9,000 past it.
//!
//! Every ratio is to `baseline`, one transaction on the arena collected
//! after every navigation, where no garbage is live: the floor.
//!
//! - `garbage`: nothing refreshed, F66's case.
//! - `checked`: the baseline arena with the flag test on, its cost when
//!   there is nothing to skip.
//! - `skip`: after a mark-only refresh, the transaction tests the flag of
//!   each dependent and skips the dead, whose entries are still listed.
//! - `pruned`: after a census (the mark, then the dead out of the reached
//!   nodes' dependents lists), the plain transaction.
//! - `mark`, `census`, `collect`: each refresh on the arena with all the
//!   abandoned screens in it, so a ratio is the refresh in transactions.
//! - `census-one`, `collect-one`: a census or a collection after one more
//!   navigation on the baseline arena, what refreshing after every
//!   navigation costs each time.
//!
//! The transactions and the mark (which leaves nothing to redo when run
//! again) run in place; the other refreshes run on a clone made in
//! `iter_batched`'s setup, handed back so its drop isn't timed.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};

use bough_experiments::rfd_0005_demand_bounded_push::{Fixture, Push, Refresh};

const ABANDONED: [usize; 3] = [100, 1_000, 9_000];

/// Screens kept live in the `app` group.
const APP_KEPT: usize = 430;

fn demand_bounded_push(c: &mut Criterion) {
    for (group, kept) in [("nav", 0), ("app", APP_KEPT)] {
        let mut g = c.benchmark_group(group);
        g.sampling_mode(SamplingMode::Flat);
        for n in ABANDONED {
            let base = Fixture::new(kept, n, Refresh::Collect).warmed(Push::All);
            let fresh = Fixture::new(kept, n, Refresh::None);
            let one = base.clone().navigated();
            let tx = |g: &mut criterion::BenchmarkGroup<_>, id: &str, f: &Fixture, push: Push| {
                let mut f = f.clone().warmed(push);
                g.bench_function(BenchmarkId::new(id, n), |b| {
                    let mut x = 0;
                    b.iter(|| {
                        x += 1;
                        black_box(f.click(black_box(x), push))
                    })
                });
            };
            let refresh =
                |g: &mut criterion::BenchmarkGroup<_>, id: &str, f: &Fixture, r: Refresh| {
                    g.bench_function(BenchmarkId::new(id, n), |b| {
                        b.iter_batched(
                            || f.clone(),
                            |mut f| (black_box(f.arena.refresh(r)), f),
                            BatchSize::LargeInput,
                        )
                    });
                };
            g.measurement_time(Duration::from_millis(300));
            tx(&mut g, "baseline", &base, Push::All);
            tx(&mut g, "garbage", &fresh, Push::All);
            tx(&mut g, "checked", &base, Push::Skip);
            tx(
                &mut g,
                "skip",
                &fresh.clone().refreshed(Refresh::Mark),
                Push::Skip,
            );
            tx(
                &mut g,
                "pruned",
                &fresh.clone().refreshed(Refresh::Census),
                Push::All,
            );
            let mut marked = fresh.clone();
            g.bench_function(BenchmarkId::new("mark", n), |b| {
                b.iter(|| black_box(marked.arena.refresh(Refresh::Mark)))
            });
            // A cloned iteration also pays the clone and its drop, up to a
            // few hundred times the census's own time, and Criterion sizes
            // iteration counts by the routine alone, so these measure for
            // less time.
            g.measurement_time(Duration::from_millis(40));
            refresh(&mut g, "census", &fresh, Refresh::Census);
            refresh(&mut g, "collect", &fresh, Refresh::Collect);
            refresh(&mut g, "census-one", &one, Refresh::Census);
            refresh(&mut g, "collect-one", &one, Refresh::Collect);
        }
        g.finish();
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(100));
    targets = demand_bounded_push
}
criterion_main!(benches);
