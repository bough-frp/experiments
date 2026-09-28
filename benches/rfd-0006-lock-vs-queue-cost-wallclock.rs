//! Wall-clock times for the lock-against-queue probe: a 500 ns, 50-node
//! propagation run bare on the owner thread (the baseline), behind a
//! `std::sync::Mutex`, and sent through RFD 6's inbox to a driver that
//! pumps.
//!
//! `single/<variant>/<n>` is everything on one thread, `n` units per
//! iteration: `baseline` calls the propagation, `lock` takes an
//! uncontended mutex around each, `queue` sends each through the inbox and
//! pumps after every unit (`n` = 1) or after a burst of 64. The instruction
//! bench counts the same runs.
//!
//! `contended/<variant>/<threads>` is one unit per iteration, spread over
//! `threads` threads timed from a barrier (`iter_custom`): `lock` is that
//! many threads hammering the mutex, `queue` that many producers sending
//! while this thread drives, and `baseline` the same units run bare on one
//! thread, which is the most any design could get from an engine that runs
//! one unit at a time. These are multi-threaded, so they are wall-clock
//! only; the instruction bench leaves them out.
//!
//! After Criterion, the bench prints the spread of waits under contention,
//! which Criterion's means hide: each locked call's wait for the lock and
//! the longest run of units one thread got in a row (F78: std's mutex is
//! unfair), and for the queue each send's time and each of the driver's
//! pops. Under `--test` it prints them from a short run, as a check only.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group};

use bough_experiments::rfd_0006_lock_vs_queue_cost::{
    BURST, Single, Spread, bare, lock_spread, locked, queue_spread, queued, run_bare, run_locked,
    run_queued,
};

const THREADS: [usize; 3] = [2, 4, 8];

fn single(c: &mut Criterion) {
    let mut g = c.benchmark_group("single");
    let mut s = Single::new();
    for n in [1, BURST] {
        // Inputs keep changing so no run repeats the last one's work.
        let mut k = 0;
        g.bench_function(BenchmarkId::new("baseline", n), |b| {
            b.iter(|| {
                k += n;
                black_box(bare(&mut s.graph, black_box(k), n))
            })
        });
        g.bench_function(BenchmarkId::new("lock", n), |b| {
            b.iter(|| {
                k += n;
                black_box(locked(&s.mutex, black_box(k), n))
            })
        });
        g.bench_function(BenchmarkId::new("queue", n), |b| {
            b.iter(|| {
                k += n;
                black_box(queued(&s.inbox, &mut s.graph, black_box(k), n, n))
            })
        });
    }
    g.finish();
}

fn contended(c: &mut Criterion) {
    let mut g = c.benchmark_group("contended");
    // Every sample is the same number of units, so a thread's start-up
    // and join, which are paid once per sample, weigh the same in each.
    g.sampling_mode(SamplingMode::Flat);
    for threads in THREADS {
        g.bench_function(BenchmarkId::new("baseline", threads), |b| {
            b.iter_custom(|units| black_box(run_bare(units)).0)
        });
        g.bench_function(BenchmarkId::new("lock", threads), |b| {
            b.iter_custom(|units| black_box(run_locked(threads, units)).0)
        });
        g.bench_function(BenchmarkId::new("queue", threads), |b| {
            b.iter_custom(|units| black_box(run_queued(threads, units)).0)
        });
    }
    g.finish();
}

/// The spread of waits under contention, printed as a table. Per-thread
/// counts: a full run, or a short one under `--test`.
fn spreads(per_thread: u64) {
    println!();
    println!("Waits under contention, ns ({per_thread} units per thread, one run each)");
    println!(
        "{:<22} {:>7} {:>7} {:>8} {:>9} {:>9}  note",
        "what", "threads", "p50", "p99", "p99.9", "max"
    );
    let row = |what: &str, threads: usize, s: Spread, note: String| {
        println!(
            "{:<22} {:>7} {:>7} {:>8} {:>9} {:>9}  {}",
            what, threads, s.p50, s.p99, s.p999, s.max, note
        );
    };
    for threads in [1].into_iter().chain(THREADS) {
        let l = lock_spread(threads, per_thread);
        row(
            "lock: wait to acquire",
            threads,
            l.wait,
            format!("longest run by one thread: {}", l.longest_streak),
        );
    }
    for producers in [1].into_iter().chain(THREADS) {
        let q = queue_spread(producers, per_thread);
        row("queue: send", producers, q.send, String::new());
        row(
            "queue: driver's pop",
            producers,
            q.pop,
            format!("longest pump: {} units", q.longest_pump),
        );
    }
}

criterion_group! {
    name = benches;
    // A unit is 500 ns, so a contended sample of a tenth of a second is
    // about 200,000 units: thread start-up (tens of microseconds) is
    // well under 1%. About two minutes in all, with the spreads.
    config = Criterion::default()
        .sample_size(30)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3));
    targets = single, contended
}

fn main() {
    let test = std::env::args().any(|a| a == "--test");
    benches();
    Criterion::default().configure_from_args().final_summary();
    let started = Instant::now();
    spreads(if test { 1_000 } else { 50_000 });
    println!("(spreads took {:.1} s)", started.elapsed().as_secs_f64());
}
