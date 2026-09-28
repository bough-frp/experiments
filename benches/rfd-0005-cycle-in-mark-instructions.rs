//! Instruction counts for the cycle-in-mark probe: one switch move checked
//! by RFD 5's upstream walk, against one transaction's mark over the same
//! graph with and without the grey check, at F50's ten thousand upstream
//! nodes and at a thousand.
//!
//! The graph is built in setup, outside what is counted, and walked and
//! marked once there, so the vectors both reuse have grown. The walk's graph
//! has the switch on its old inner and the bench moves it and walks; the
//! marks' graph has the switch moved already and the bench marks from the
//! input above the upstream nodes, which orders them, the new inner, the
//! switch and the hundred nodes below it. `mark_plain` is the baseline
//! the grey check adds to. Each bench hands the graph back, so its drop
//! runs in teardown, uncounted.

use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use bough_experiments::rfd_0005_cycle_in_mark::F50;

const DOWNSTREAM: u32 = 100;

fn unmoved(upstream: u32) -> F50 {
    let mut f = F50::new(upstream, DOWNSTREAM);
    f.warm();
    f
}

fn moved(upstream: u32) -> F50 {
    let mut f = F50::moved(upstream, DOWNSTREAM);
    f.warm();
    f
}

fn drop_graph<T>(out: (T, F50)) {
    black_box(out.0);
}

#[library_benchmark]
#[benches::upstream(args = [1_000, 10_000], setup = unmoved, teardown = drop_graph)]
fn walk_one_move(mut f: F50) -> (bool, F50) {
    (black_box(f.move_and_walk()), f)
}

#[library_benchmark]
#[benches::upstream(args = [1_000, 10_000], setup = moved, teardown = drop_graph)]
fn mark_plain(mut f: F50) -> (usize, F50) {
    f.mark::<false>().expect("the plain mark reports nothing");
    (black_box(f.g.order.len()), f)
}

#[library_benchmark]
#[benches::upstream(args = [1_000, 10_000], setup = moved, teardown = drop_graph)]
fn mark_grey(mut f: F50) -> (usize, F50) {
    f.mark::<true>().expect("F50's move is acyclic");
    (black_box(f.g.order.len()), f)
}

library_benchmark_group!(
    name = cycle_in_mark;
    benchmarks = walk_one_move, mark_plain, mark_grey
);

main!(library_benchmark_groups = cycle_in_mark);
