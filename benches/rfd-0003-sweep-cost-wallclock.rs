//! Wall-clock times for the sweep-cost probe: one collection of the arena
//! at a thousand, ten thousand and a hundred thousand slots, with a tenth
//! (`live10`) and nine tenths (`live90`) of it surviving, split into its
//! mark, its sweep (free the unmarked, bump, push on the free list) and its
//! prune (the dead out of the survivors' dependents lists), against one
//! transaction over one screen.
//!
//! The baseline is that transaction, the mark-and-evaluate a unit of this
//! UI does over the region its input reaches (21 nodes, the same screen at
//! every size), run on the collected arena. The trigger's question is how
//! many units a collection costs, so the ratio a note quotes is collections
//! in units. `pass` is a pass over the collected arena that reads each
//! slot's liveness and stamp once, the per-slot floor under the sweep.
//!
//! Every collection and every part of one runs on a clone of one fixture,
//! made in `iter_batched`'s setup, and hands the clone back so its drop is
//! not timed either; the sweep's clone has its mark done already, and the
//! prune's its mark and sweep.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};

use bough_experiments::rfd_0003_sweep_cost::Fixture;

const SLOTS: [usize; 3] = [1_000, 10_000, 100_000];

fn sweep_cost(c: &mut Criterion) {
    for live in [10, 90] {
        let mut g = c.benchmark_group(format!("live{live}"));
        g.sampling_mode(SamplingMode::Flat);
        for slots in SLOTS {
            let fresh = Fixture::new(slots, live);
            let marked = fresh.clone().marked();
            let swept = fresh.clone().swept();
            let mut collected = fresh.clone().collected();
            let input = collected.kept.input;
            g.bench_function(BenchmarkId::new("baseline", slots), |b| {
                let mut x = 0;
                b.iter(|| {
                    x += 1;
                    black_box(collected.arena.transaction(input, black_box(x)))
                })
            });
            g.bench_function(BenchmarkId::new("collect", slots), |b| {
                b.iter_batched(
                    || fresh.clone(),
                    |mut f| (black_box(f.arena.collect()), f),
                    BatchSize::LargeInput,
                )
            });
            g.bench_function(BenchmarkId::new("mark", slots), |b| {
                b.iter_batched(
                    || fresh.clone(),
                    |mut f| (black_box(f.arena.mark_roots()), f),
                    BatchSize::LargeInput,
                )
            });
            g.bench_function(BenchmarkId::new("sweep", slots), |b| {
                b.iter_batched(
                    || marked.clone(),
                    |mut f| (black_box(f.arena.sweep()), f),
                    BatchSize::LargeInput,
                )
            });
            g.bench_function(BenchmarkId::new("prune", slots), |b| {
                b.iter_batched(
                    || swept.clone(),
                    |mut f| {
                        f.arena.prune();
                        f
                    },
                    BatchSize::LargeInput,
                )
            });
            g.bench_function(BenchmarkId::new("pass", slots), |b| {
                b.iter(|| black_box(collected.arena.touch_each_slot()))
            });
        }
        g.finish();
    }
}

criterion_group! {
    name = benches;
    // Criterion sizes its iteration counts by the routine's time alone,
    // but every cloned iteration also pays the clone and its drop, up to a
    // hundred times the routine's time at a hundred thousand slots (a mark
    // of a tenth of it, a prune). So the sampling is flat and the times
    // short, which keeps the whole bench near five minutes.
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_millis(300));
    targets = sweep_cost
}
criterion_main!(benches);
