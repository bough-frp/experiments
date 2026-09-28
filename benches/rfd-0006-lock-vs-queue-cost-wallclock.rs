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
//! `footprint-<bytes>/<variant>/<threads>` asks where the contended lock's
//! extra time goes. The unit is the `Footprint` stand-in: the same chain
//! of work, plus one read-modify-write of every line of `bytes` of state
//! (0 B, 64 B, 1,216 B as the graph's, 16 KiB, 256 KiB). One unit per
//! iteration over `threads` threads, as in `contended`: `baseline` bare on
//! one thread at that footprint, `lock` std's mutex, `ticket` a fair spin
//! lock that never parks and hands off every unit, and `queue` producers
//! sending to a driver. The ticket lock's extra over bare against `bytes`
//! gives migration per hand-off (slope) and a bare hand-off (intercept);
//! the mutex's extra, over its share of units that were hand-offs (printed
//! after), against the ticket lock's, gives the futex. Compare extras in
//! absolute time across footprints, not ratios: the bare unit grows with
//! the footprint too.
//!
//! `footprint-<bytes>-<placement>/<variant>/<threads>` is the same at 2
//! and 4 threads with each thread pinned to its own physical core: `ccx1`
//! all in one core complex (CPUs 0, 2, 4, 6), `ccx2` alternating between
//! the two (0, 8, 2, 10), so a hand-off stays in one L3 or crosses the
//! Infinity Fabric. `baseline` is bare on a thread pinned to the first of
//! those CPUs, `lock` std's mutex, `ticket` the ticket lock. The ticket
//! lock's slope against `bytes` in each placement splits the migration cost
//! per line into same-CCX and cross-CCX; the unpinned groups mix the two.
//! At 0 B and 256 KiB the pinned groups also have `futex`: `FutexMutex`,
//! std's mutex copied line for line with each futex call counted and
//! timed, so its extra can be set beside `lock`'s and beside the time its
//! wakes take (the table after).
//!
//! After Criterion, the bench prints the spread of waits under contention,
//! which Criterion's means hide: each locked call's wait for the lock and
//! the longest run of units one thread got in a row (F78: std's mutex is
//! unfair), and for the queue each send's time and each of the driver's
//! pops. Under `--test` it prints them from a short run, as a check only.
//! Then, for each footprint, how often each lock changed threads: a unit
//! can only migrate the state when the lock went to another thread, and
//! std's mutex is unfair, so its extra per unit is its extra per hand-off
//! times the share of units that were one. The same table follows for the
//! pinned placements, with the workers' sleeps (voluntary context
//! switches), since pinning can change how often std's mutex parks. Last,
//! the copy's futex calls in each placement, at 0 B and 256 KiB: wakes and
//! woken sleeps per unit, and per call the time inside `futex_wake`,
//! inside `futex_wait`, and from a wake's send to the woken thread running
//! (wake-to-run), which says whether a wake crossing core complexes is
//! what the mutex's `ccx2` extra over `ccx1` is.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group};

use bough_experiments::rfd_0006_lock_vs_queue_cost::{
    BURST, FOOTPRINTS, Footprint, LockKind, PLACEMENTS, Single, Spread, bare, handoffs,
    handoffs_on, lock_spread, locked, queue_spread, queued, run_bare, run_bare_on, run_bare_pinned,
    run_futex_pinned, run_locked, run_locked_on, run_locked_pinned, run_queued, run_queued_on,
    run_ticket_on, run_ticket_pinned,
};

const THREADS: [usize; 3] = [2, 4, 8];

/// Threads in the pinned runs: at most 4 fit one CCX a core each.
const PINNED_THREADS: [usize; 2] = [2, 4];

/// Footprints the pinned `futex` variant takes: the hand-off alone, and
/// the state too big to move.
fn futex_footprint(bytes: usize) -> bool {
    bytes == 0 || bytes == 256 * 1024
}

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

fn footprint(c: &mut Criterion) {
    for (bytes, label) in FOOTPRINTS {
        let mut g = c.benchmark_group(format!("footprint-{label}"));
        g.sampling_mode(SamplingMode::Flat);
        // Sixty benchmarks here: two seconds each keeps the whole bench
        // under ten minutes.
        g.measurement_time(Duration::from_secs(2));
        // A fresh state per sample, built outside the timed run; each run
        // times from its barrier.
        let fresh = || Footprint::new(bytes);
        for threads in THREADS {
            g.bench_function(BenchmarkId::new("baseline", threads), |b| {
                b.iter_custom(|units| black_box(run_bare_on(fresh(), units)).0)
            });
            g.bench_function(BenchmarkId::new("lock", threads), |b| {
                b.iter_custom(|units| black_box(run_locked_on(fresh(), threads, units)).0)
            });
            g.bench_function(BenchmarkId::new("ticket", threads), |b| {
                b.iter_custom(|units| black_box(run_ticket_on(fresh(), threads, units)).0)
            });
            g.bench_function(BenchmarkId::new("queue", threads), |b| {
                b.iter_custom(|units| black_box(run_queued_on(fresh(), threads, units)).0)
            });
        }
        g.finish();
    }
}

fn footprint_pinned(c: &mut Criterion) {
    for (bytes, label) in FOOTPRINTS {
        for (placement, cpus) in PLACEMENTS {
            let mut g = c.benchmark_group(format!("footprint-{label}-{placement}"));
            g.sampling_mode(SamplingMode::Flat);
            // Sixty more benchmarks, and eight `futex`, at the unpinned
            // groups' two seconds.
            g.measurement_time(Duration::from_secs(2));
            let fresh = || Footprint::new(bytes);
            for threads in PINNED_THREADS {
                let cpus = &cpus[..threads];
                g.bench_function(BenchmarkId::new("baseline", threads), |b| {
                    b.iter_custom(|units| black_box(run_bare_pinned(fresh(), cpus[0], units)).0)
                });
                g.bench_function(BenchmarkId::new("lock", threads), |b| {
                    b.iter_custom(|units| black_box(run_locked_pinned(fresh(), cpus, units)).0)
                });
                g.bench_function(BenchmarkId::new("ticket", threads), |b| {
                    b.iter_custom(|units| black_box(run_ticket_pinned(fresh(), cpus, units)).0)
                });
                if futex_footprint(bytes) {
                    g.bench_function(BenchmarkId::new("futex", threads), |b| {
                        b.iter_custom(|units| black_box(run_futex_pinned(fresh(), cpus, units)).0)
                    });
                }
            }
            g.finish();
        }
    }
}

/// How often each lock changed threads, per footprint, as a table.
fn handoff_table(per_thread: u64) {
    println!();
    println!("Lock hand-offs ({per_thread} units per thread, one run each, untimed)");
    println!(
        "{:<8} {:<6} {:>7} {:>9} {:>9} {:>9}",
        "bytes", "lock", "threads", "units", "changes", "longest"
    );
    for (bytes, label) in FOOTPRINTS {
        for kind in [LockKind::Mutex, LockKind::Ticket] {
            for threads in THREADS {
                let h = handoffs(kind, bytes, threads, per_thread);
                println!(
                    "{:<8} {:<6} {:>7} {:>9} {:>9} {:>9}",
                    label,
                    kind.name(),
                    threads,
                    h.units,
                    h.changes,
                    h.longest_streak
                );
            }
        }
    }
    println!();
    println!("Pinned: the same, with the workers' sleeps (voluntary context switches)");
    println!(
        "{:<8} {:<5} {:<6} {:>7} {:>9} {:>9} {:>9} {:>9}",
        "bytes", "where", "lock", "threads", "units", "changes", "longest", "sleeps"
    );
    for (bytes, label) in FOOTPRINTS {
        for (placement, cpus) in PLACEMENTS {
            for kind in [LockKind::Mutex, LockKind::Ticket] {
                for threads in PINNED_THREADS {
                    let h = handoffs_on(kind, bytes, threads, &cpus[..threads], per_thread);
                    println!(
                        "{:<8} {:<5} {:<6} {:>7} {:>9} {:>9} {:>9} {:>9}",
                        label,
                        placement,
                        kind.name(),
                        threads,
                        h.units,
                        h.changes,
                        h.longest_streak,
                        h.sleeps
                    );
                }
            }
        }
    }
}

/// The copy of std's mutex pinned: its futex calls per unit and the time
/// each takes, per placement. One untimed-by-Criterion run each; the times
/// are the calls' own, read inside the run.
fn futex_pinned_table(per_thread: u64) {
    println!();
    println!(
        "Pinned copy of std's mutex: futex calls per unit, and ns per call ({per_thread} units per thread, one run each)"
    );
    println!(
        "{:<8} {:<5} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "bytes", "where", "threads", "wakes/u", "woken/u", "slept/u", "wake", "wait", "to-run"
    );
    let per_call = |ns: u64, calls: u64| {
        if calls == 0 {
            "-".to_string()
        } else {
            format!("{:.0}", ns as f64 / calls as f64)
        }
    };
    for (bytes, label) in FOOTPRINTS {
        if !futex_footprint(bytes) {
            continue;
        }
        for (placement, cpus) in PLACEMENTS {
            for threads in PINNED_THREADS {
                let h = handoffs_on(
                    LockKind::Futex,
                    bytes,
                    threads,
                    &cpus[..threads],
                    per_thread,
                );
                let (f, u) = (h.futex, h.units as f64);
                println!(
                    "{:<8} {:<5} {:>7} {:>8.3} {:>8.3} {:>8.3} {:>8} {:>8} {:>8}",
                    label,
                    placement,
                    threads,
                    f.wakes as f64 / u,
                    f.woken as f64 / u,
                    f.slept as f64 / u,
                    per_call(f.wake_ns, f.wakes),
                    per_call(f.wait_ns, f.waits),
                    per_call(f.wake_to_run_ns, f.slept),
                );
            }
        }
    }
    println!("wake: inside futex_wake. wait: inside futex_wait, asleep behind the holder");
    println!("included. to-run: a wake's send to the woken thread's return, over waits that");
    println!("slept and were woken (exact at 2 threads, reads low at 4).");
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
    // well under 1%. About two minutes in all, with the spreads, before
    // the footprint groups, which set their own measurement time.
    config = Criterion::default()
        .sample_size(30)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3));
    targets = single, contended, footprint, footprint_pinned
}

fn main() {
    let test = std::env::args().any(|a| a == "--test");
    benches();
    Criterion::default().configure_from_args().final_summary();
    let started = Instant::now();
    spreads(if test { 1_000 } else { 50_000 });
    println!("(spreads took {:.1} s)", started.elapsed().as_secs_f64());
    let started = Instant::now();
    handoff_table(if test { 200 } else { 5_000 });
    println!("(hand-offs took {:.1} s)", started.elapsed().as_secs_f64());
    let started = Instant::now();
    futex_pinned_table(if test { 200 } else { 20_000 });
    println!(
        "(pinned futex calls took {:.1} s)",
        started.elapsed().as_secs_f64()
    );
}
