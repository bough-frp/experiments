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

use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use bough_experiments::rfd_0006_lock_vs_queue_cost::{Single, bare, locked, queued};

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

main!(library_benchmark_groups = lock_vs_queue);
