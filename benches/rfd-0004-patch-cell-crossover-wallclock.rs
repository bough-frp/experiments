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
//! After those, `btree` in the `vec-*` groups (the delta design over
//! counted B-trees) and `fullylazy` in the `map-*` groups (the source map
//! deferred too, raw upserts buffered until a read). The `vec-*` groups
//! also take n = 1,000,000, every variant. Last, `fused` in the `map-*`
//! groups (the delta design eager, each upsert one insert on the source
//! whose old value is the filter's retraction) and `btree-shared` in the
//! `vec-*` groups (one B-tree holding source and mapped values together).
//!
//! Each fixture is checked first to give the same reads under both
//! designs, and warmed for a cycle.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use bough_experiments::rfd_0004_patch_cell_crossover::{
    APPEND, COMPOSE_K, CYCLE, Compose, MILLION, Maps, ONE, RARE_READ, SIZES, TWO, Variant, Vecs,
    Workload,
};

fn vec_group(c: &mut Criterion, w: Workload) {
    let mut g = c.benchmark_group(format!("vec-{}", w.name));
    for &n in SIZES.iter().chain([&MILLION]) {
        let mut fixture = Vecs::new(n, w);
        for _ in 0..CYCLE {
            let read = fixture.step(Variant::Baseline);
            assert_eq!(read, fixture.step(Variant::Delta));
            assert_eq!(read, fixture.step(Variant::Rope));
            assert_eq!(read, fixture.step(Variant::BTree));
            assert_eq!(read, fixture.step(Variant::BTreeShared));
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
            assert_eq!(read, fixture.step(Variant::FullyLazy));
            assert_eq!(read, fixture.step(Variant::Fused));
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
    // 275 benchmarks. At 0.3 s warm-up and 1 s measuring, the last run's
    // 222 took about eight and a half minutes, about 2.3 s each with the
    // fixtures and the slow ones (map at 100k, vec baseline at 1M); at
    // 0.2 s and 0.8 s, 275 should take about eight and a half again.
    config = Criterion::default()
        .sample_size(50)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(800));
    targets = patch_cell_crossover
}
criterion_main!(benches);
