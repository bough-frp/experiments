//! Instruction counts for the patch-cell crossover, open question 9 and
//! RFD 4: the same comparisons as the wall-clock bench.
//!
//! Each bench runs one cycle, `CYCLE` (64) instants, of one design on one
//! fixture: `baseline` (a cell of the collection, derived cells read-through
//! and recomputed on read) or `delta` (a patch-carrying cell, derived cells
//! updated from the patch). Divide by 64 for a cost per instant. The
//! parameter is the collection's size n. Workloads: `one` (one source, read
//! every instant), `two` (two sources composed in one instant), `rare` (one
//! source, read every 16th instant) and, for `Vec` only, `append` (edits at
//! the end instead of at a random index). `compose` counts 64 compositions
//! of k Z-sets against 64 concatenations of k commands, the parameter k.
//!
//! The fixture is built in `setup` and both designs run a cycle on it
//! there, so memos, capacities and the hash maps are at their steady state,
//! and the counted cycle starts where a cycle starts. It's handed back so
//! its drop isn't counted.

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use bough_experiments::rfd_0004_patch_cell_crossover::{
    APPEND, CYCLE, Compose, Maps, ONE, RARE_READ, TWO, Variant, Vecs, Workload,
};

fn vecs(n: usize, w: Workload) -> Vecs {
    let mut f = Vecs::new(n, w);
    for v in Variant::ALL {
        f.steps(v, CYCLE);
    }
    f
}

fn vec_one(n: usize) -> Vecs {
    vecs(n, ONE)
}
fn vec_two(n: usize) -> Vecs {
    vecs(n, TWO)
}
fn vec_rare(n: usize) -> Vecs {
    vecs(n, RARE_READ)
}
fn vec_append(n: usize) -> Vecs {
    vecs(n, APPEND)
}

fn maps(n: usize, w: Workload) -> Maps {
    let mut f = Maps::new(n, w);
    for v in Variant::ALL {
        f.steps(v, CYCLE);
    }
    f
}

fn map_one(n: usize) -> Maps {
    maps(n, ONE)
}
fn map_two(n: usize) -> Maps {
    maps(n, TWO)
}
fn map_rare(n: usize) -> Maps {
    maps(n, RARE_READ)
}

fn compose(k: usize) -> Compose {
    let mut c = Compose::new(k);
    for v in Variant::ALL {
        c.run(v);
    }
    c
}

fn drop_out<T>(out: (u64, T)) {
    black_box(out.0);
}

#[library_benchmark]
#[benches::one(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = vec_one, teardown = drop_out)]
#[benches::two(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = vec_two, teardown = drop_out)]
#[benches::rare(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = vec_rare, teardown = drop_out)]
#[benches::append(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = vec_append, teardown = drop_out)]
fn vec_baseline(mut f: Vecs) -> (u64, Vecs) {
    (black_box(f.steps(Variant::Baseline, CYCLE)), f)
}

#[library_benchmark]
#[benches::one(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = vec_one, teardown = drop_out)]
#[benches::two(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = vec_two, teardown = drop_out)]
#[benches::rare(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = vec_rare, teardown = drop_out)]
#[benches::append(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = vec_append, teardown = drop_out)]
fn vec_delta(mut f: Vecs) -> (u64, Vecs) {
    (black_box(f.steps(Variant::Delta, CYCLE)), f)
}

#[library_benchmark]
#[benches::one(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = map_one, teardown = drop_out)]
#[benches::two(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = map_two, teardown = drop_out)]
#[benches::rare(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = map_rare, teardown = drop_out)]
fn map_baseline(mut f: Maps) -> (u64, Maps) {
    (black_box(f.steps(Variant::Baseline, CYCLE)), f)
}

#[library_benchmark]
#[benches::one(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = map_one, teardown = drop_out)]
#[benches::two(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = map_two, teardown = drop_out)]
#[benches::rare(args = [3, 10, 30, 100, 1_000, 10_000, 100_000], setup = map_rare, teardown = drop_out)]
fn map_delta(mut f: Maps) -> (u64, Maps) {
    (black_box(f.steps(Variant::Delta, CYCLE)), f)
}

fn run_compose(mut c: Compose, v: Variant) -> (u64, Compose) {
    let mut acc = 0u64;
    for _ in 0..CYCLE {
        acc += black_box(c.run(v)) as u64;
    }
    (acc, c)
}

#[library_benchmark]
#[benches::k(args = [1, 2, 4, 16, 64], setup = compose, teardown = drop_out)]
fn compose_baseline(c: Compose) -> (u64, Compose) {
    run_compose(c, Variant::Baseline)
}

#[library_benchmark]
#[benches::k(args = [1, 2, 4, 16, 64], setup = compose, teardown = drop_out)]
fn compose_delta(c: Compose) -> (u64, Compose) {
    run_compose(c, Variant::Delta)
}

library_benchmark_group!(
    name = patch_cell_crossover;
    benchmarks = vec_baseline, vec_delta, map_baseline, map_delta, compose_baseline, compose_delta
);

main!(library_benchmark_groups = patch_cell_crossover);
