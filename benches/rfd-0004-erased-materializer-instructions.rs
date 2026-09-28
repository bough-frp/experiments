//! Instruction counts for the erased-materializer probe: a thousand
//! transactions, one event each, through a chain of three adapters,
//! `input.map(f).filter(p).snapshot(cell, g).hold(b, 0)`, with the chain
//! handed to its materializer four ways (`baseline`, `boxed`, `fnptr`,
//! `flat`; the wall-clock bench says what each is). Divide by a thousand
//! for instructions per event.
//!
//! The graph is built in setup and handed back for teardown, so neither its
//! build nor its drop is counted.

use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use bough_experiments::rfd_0004_erased_materializer::{Design, ThreeAdapters};

const EVENTS: u64 = 1000;

fn drop_chain(out: (u64, ThreeAdapters)) {
    black_box(out.0);
}

#[library_benchmark]
#[bench::three(args = (Design::Baseline), setup = ThreeAdapters::new, teardown = drop_chain)]
fn baseline(mut chain: ThreeAdapters) -> (u64, ThreeAdapters) {
    (black_box(chain.events(EVENTS)), chain)
}

#[library_benchmark]
#[bench::three(args = (Design::Boxed), setup = ThreeAdapters::new, teardown = drop_chain)]
fn boxed(mut chain: ThreeAdapters) -> (u64, ThreeAdapters) {
    (black_box(chain.events(EVENTS)), chain)
}

#[library_benchmark]
#[bench::three(args = (Design::Fnptr), setup = ThreeAdapters::new, teardown = drop_chain)]
fn fnptr(mut chain: ThreeAdapters) -> (u64, ThreeAdapters) {
    (black_box(chain.events(EVENTS)), chain)
}

#[library_benchmark]
#[bench::three(args = (Design::Flat), setup = ThreeAdapters::new, teardown = drop_chain)]
fn flat(mut chain: ThreeAdapters) -> (u64, ThreeAdapters) {
    (black_box(chain.events(EVENTS)), chain)
}

library_benchmark_group!(
    name = event;
    benchmarks = baseline, boxed, fnptr, flat
);

main!(library_benchmark_groups = event);
