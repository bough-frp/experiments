//! Instruction counts for the lock-against-queue probe, RFD 6: the
//! single-threaded half of the wall-clock bench, `n` units of the 50-node
//! propagation per bench, at `n` = 1 and 64. Divide by `n` for a cost per
//! unit.
//!
//! - `baseline`: the owner thread calls the propagation.
//! - `lock`: an uncontended `std::sync::Mutex` locked around each unit.
//! - `queue`: each unit boxed, sent through RFD 6's inbox, and run by a
//!   pump, which comes after every unit at `n` = 1 and after all 64 at
//!   `n` = 64. The driver's waker unparks this thread, as a thread
//!   driver's would, and is called once per pump.
//!
//! The contended variants run on several threads and are left out: an
//! instruction count of a thread that waits on a lock or parks says
//! nothing about how long it waited. They are in the wall-clock bench.
//!
//! The fixture is built and warmed in `setup` (every variant has run once
//! and the inbox has grown to a burst), and handed back so its drop isn't
//! counted.
//!
//! The `footprint` group is the same comparison on the probe's
//! `Footprint` stand-in, the chain's fixed work plus a read-modify-write of
//! every line of `bytes` of state, at `(bytes, n)`: `bytes` of 0, 64,
//! 1,216 (the graph's 1.2 KB), 16 KiB and 256 KiB, and `n` of 1 and 64.
//! It adds `ticket`, an uncontended ticket lock, the fair spin lock the
//! wall-clock bench sets beside std's to take the futex out. On one thread nothing migrates,
//! so what this shows is that each variant's own cost doesn't grow with
//! the footprint, and the cache simulation's misses as the state outgrows
//! L1; what the footprint does to a contended lock is in the wall-clock
//! bench.

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use bough_experiments::rfd_0006_lock_vs_queue_cost::{
    Footprint, Single, bare, locked, queued, ticketed,
};

fn fixture(n: u64) -> (Single, u64) {
    (Single::new(), n)
}

fn drop_fixture(out: (u64, Single)) {
    black_box(out.0);
}

#[library_benchmark]
#[benches::single(args = [1, 64], setup = fixture, teardown = drop_fixture)]
fn baseline((mut s, n): (Single, u64)) -> (u64, Single) {
    (black_box(bare(&mut s.graph, black_box(1), n)), s)
}

#[library_benchmark]
#[benches::single(args = [1, 64], setup = fixture, teardown = drop_fixture)]
fn lock((s, n): (Single, u64)) -> (u64, Single) {
    (black_box(locked(&s.mutex, black_box(1), n)), s)
}

#[library_benchmark]
#[benches::single(args = [1, 64], setup = fixture, teardown = drop_fixture)]
fn queue((mut s, n): (Single, u64)) -> (u64, Single) {
    (
        black_box(queued(&s.inbox, &mut s.graph, black_box(1), n, n)),
        s,
    )
}

library_benchmark_group!(
    name = lock_vs_queue;
    benchmarks = baseline, lock, queue
);

fn footprint_fixture(bytes: usize, n: u64) -> (Single<Footprint>, u64) {
    (Single::of(|| Footprint::new(bytes)), n)
}

fn drop_footprint(out: (u64, Single<Footprint>)) {
    black_box(out.0);
}

#[library_benchmark]
#[benches::one(args = [(0, 1), (64, 1), (1216, 1), (16384, 1), (262144, 1)], setup = footprint_fixture, teardown = drop_footprint)]
#[benches::burst(args = [(0, 64), (64, 64), (1216, 64), (16384, 64), (262144, 64)], setup = footprint_fixture, teardown = drop_footprint)]
fn fp_baseline((mut s, n): (Single<Footprint>, u64)) -> (u64, Single<Footprint>) {
    (black_box(bare(&mut s.graph, black_box(1), n)), s)
}

#[library_benchmark]
#[benches::one(args = [(0, 1), (64, 1), (1216, 1), (16384, 1), (262144, 1)], setup = footprint_fixture, teardown = drop_footprint)]
#[benches::burst(args = [(0, 64), (64, 64), (1216, 64), (16384, 64), (262144, 64)], setup = footprint_fixture, teardown = drop_footprint)]
fn fp_lock((s, n): (Single<Footprint>, u64)) -> (u64, Single<Footprint>) {
    (black_box(locked(&s.mutex, black_box(1), n)), s)
}

#[library_benchmark]
#[benches::one(args = [(0, 1), (64, 1), (1216, 1), (16384, 1), (262144, 1)], setup = footprint_fixture, teardown = drop_footprint)]
#[benches::burst(args = [(0, 64), (64, 64), (1216, 64), (16384, 64), (262144, 64)], setup = footprint_fixture, teardown = drop_footprint)]
fn fp_ticket((s, n): (Single<Footprint>, u64)) -> (u64, Single<Footprint>) {
    (black_box(ticketed(&s.ticket, black_box(1), n)), s)
}

#[library_benchmark]
#[benches::one(args = [(0, 1), (64, 1), (1216, 1), (16384, 1), (262144, 1)], setup = footprint_fixture, teardown = drop_footprint)]
#[benches::burst(args = [(0, 64), (64, 64), (1216, 64), (16384, 64), (262144, 64)], setup = footprint_fixture, teardown = drop_footprint)]
fn fp_queue((mut s, n): (Single<Footprint>, u64)) -> (u64, Single<Footprint>) {
    (
        black_box(queued(&s.inbox, &mut s.graph, black_box(1), n, n)),
        s,
    )
}

library_benchmark_group!(
    name = footprint;
    benchmarks = fp_baseline, fp_lock, fp_ticket, fp_queue
);

main!(library_benchmark_groups = lock_vs_queue, footprint);
