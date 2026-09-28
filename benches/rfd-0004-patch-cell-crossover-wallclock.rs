//! Wall-clock times for the patch-cell crossover, open question 9 and
//! RFD 4: one instant per iteration, cycling through the fixture's 64
//! instants, the same the instruction counts run.
//!
//! In each group the baseline is RFD 4's cell of the collection, an
//! in-place `accumulate_mut` state with read-through derived cells that
//! recompute on the first read after a step, and `delta` is a cell
//! carrying patches with derived cells updated from each patch. Groups
//! `vec-*` (a `Vec<u64>`, `map` then `sum`) and `map-*` (a
//! `HashMap<u64, u64>`, filter then count) take the collection's size n.
//! Workloads: `one` (one source, read every instant), `two` (two sources
//! composed in one instant), `rare` (read every 16th instant), and
//! `vec-append` (edits at the end). `compose` takes k, the number of
//! Z-sets consolidated in one instant, against concatenating k commands.
//!
//! Added later, beside the earlier IDs: `rope` in the `vec-*` groups (the
//! delta design over ropes) and `lazy` in the `map-*` groups (the delta
//! design with the derived cells buffering Z-sets until a read).
//!
//! Each fixture is checked first to give the same reads under both
//! designs, and warmed for a cycle.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use bough_experiments::rfd_0004_patch_cell_crossover::{
    APPEND, COMPOSE_K, CYCLE, Compose, Maps, ONE, RARE_READ, SIZES, TWO, Variant, Vecs, Workload,
};

fn vec_group(c: &mut Criterion, w: Workload) {
    let mut g = c.benchmark_group(format!("vec-{}", w.name));
    for &n in &SIZES {
        let mut fixture = Vecs::new(n, w);
        for _ in 0..CYCLE {
            let read = fixture.step(Variant::Baseline);
            assert_eq!(read, fixture.step(Variant::Delta));
            assert_eq!(read, fixture.step(Variant::Rope));
        }
        for v in Variant::VEC {
            let mut f = fixture.clone();
            g.bench_function(BenchmarkId::new(v.name(), n), |b| {
                b.iter(|| black_box(f.step(v)))
            });
        }
    }
    g.finish();
}

fn map_group(c: &mut Criterion, w: Workload) {
    let mut g = c.benchmark_group(format!("map-{}", w.name));
    for &n in &SIZES {
        let mut fixture = Maps::new(n, w);
        for _ in 0..CYCLE {
            let read = fixture.step(Variant::Baseline);
            assert_eq!(read, fixture.step(Variant::Delta));
            assert_eq!(read, fixture.step(Variant::Lazy));
        }
        for v in Variant::MAP {
            let mut f = fixture.clone();
            g.bench_function(BenchmarkId::new(v.name(), n), |b| {
                b.iter(|| black_box(f.step(v)))
            });
        }
    }
    g.finish();
}

fn compose_group(c: &mut Criterion) {
    let mut g = c.benchmark_group("compose");
    for &k in &COMPOSE_K {
        for v in Variant::ALL {
            let mut f = Compose::new(k);
            g.bench_function(BenchmarkId::new(v.name(), k), |b| {
                b.iter(|| black_box(f.run(v)))
            });
        }
    }
    g.finish();
}

fn patch_cell_crossover(c: &mut Criterion) {
    for w in [ONE, TWO, RARE_READ, APPEND] {
        vec_group(c, w);
    }
    for w in [ONE, TWO, RARE_READ] {
        map_group(c, w);
    }
    compose_group(c);
}

criterion_group! {
    name = benches;
    // 157 benchmarks at about 1.5 s each, the slowest (map at 100k) a few
    // seconds more: about six minutes.
    config = Criterion::default()
        .sample_size(50)
        .warm_up_time(Duration::from_millis(300))
        .measurement_time(Duration::from_secs(1));
    targets = patch_cell_crossover
}
criterion_main!(benches);
