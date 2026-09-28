//! How often does a contended `std::sync::Mutex` call into the kernel, per
//! unit, and is that where its extra time goes?
//!
//! The footprint sweep in the probe's wall-clock bench found the contended
//! mutex costing 1.1–1.4 µs a unit over bare at 256 KiB of state, where it
//! almost never changes threads, so almost nothing migrates. The guess was
//! a `futex_wake` on every unlock while other threads sleep. This counts
//! it, on the probe's `Footprint` stand-in at 0 B and 256 KiB, by 2, 4 and
//! 8 threads each running units back to back:
//!
//! - `mutex`, std's own: each worker's voluntary context switches
//!   (`getrusage(RUSAGE_THREAD)`), which are its futex sleeps. A sleep is
//!   ended by a wake, so sleeps per unit bound the wakes that did anything.
//! - `futex`, `FutexMutex`, std's futex mutex copied with its syscalls
//!   counted: `futex_wait` calls, `futex_wake` calls, and the wakes that
//!   woke a thread.
//!
//! Three runs of each, as the counts vary run to run. Counts, not times.
//!
//! Two other modes, neither for `results/`:
//!
//! - `--timed`: the same runs timed, with the time spent inside
//!   `futex_wake`, to set the counts beside the bench's extras. Wall-clock,
//!   so indicative only on a busy machine.
//! - `--std <bytes> <threads>`: one run of std's mutex and nothing else,
//!   for `strace -f -c -e trace=futex` to count its futex calls directly.

use std::time::Instant;

use bough_experiments::rfd_0006_lock_vs_queue_cost::{
    Footprint, Handoffs, LockKind, handoffs, run_bare_on,
};

const FOOTPRINTS: [(usize, &str); 2] = [(0, "0B"), (256 * 1024, "256KiB")];
const THREADS: [usize; 3] = [2, 4, 8];
const PER_THREAD: u64 = 20_000;
const RUNS: usize = 3;

fn per(n: u64, units: u64) -> f64 {
    n as f64 / units as f64
}

fn counts() {
    println!("Futex calls per unit, contended ({PER_THREAD} units per thread, {RUNS} runs each)");
    println!();
    println!(
        "{:<7} {:<6} {:>7} {:>4} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "bytes", "lock", "threads", "run", "changes", "sleeps", "waits", "wakes", "woken"
    );
    // For the verdict: the copy's wakes per unit at 256 KiB, and std's
    // sleeps per unit there.
    let (mut wakes_256, mut sleeps_256) = (Vec::new(), Vec::new());
    for (bytes, label) in FOOTPRINTS {
        for kind in [LockKind::Mutex, LockKind::Futex] {
            for threads in THREADS {
                for run in 1..=RUNS {
                    let h = handoffs(kind, bytes, threads, PER_THREAD);
                    let u = h.units;
                    let futex = |n: u64| match kind {
                        LockKind::Futex => format!("{:.3}", per(n, u)),
                        _ => "-".to_string(),
                    };
                    println!(
                        "{:<7} {:<6} {:>7} {:>4} {:>8.3} {:>8.3} {:>8} {:>8} {:>8}",
                        label,
                        kind.name(),
                        threads,
                        run,
                        per(h.changes, u),
                        per(h.sleeps, u),
                        futex(h.futex.waits),
                        futex(h.futex.wakes),
                        futex(h.futex.woken),
                    );
                    if bytes > 0 {
                        match kind {
                            LockKind::Futex => wakes_256.push(per(h.futex.wakes, u)),
                            _ => sleeps_256.push(per(h.sleeps, u)),
                        }
                    }
                }
            }
        }
    }
    let range = |v: &[f64]| {
        let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = v.iter().copied().fold(0.0, f64::max);
        format!("{lo:.2}–{hi:.2}")
    };
    println!();
    println!("Per unit: every column is a count divided by the units run.");
    println!("changes: the lock went to another thread. sleeps: voluntary context");
    println!("switches of the workers. waits, wakes: futex_wait and futex_wake syscalls");
    println!("of the copy; woken: wakes that woke a thread. '-': not countable for std's.");
    println!();
    println!(
        "Verdict: at 256 KiB, the copy of std's mutex made {} futex_wake calls per unit \
         over 2-8 threads, and std's own mutex's workers slept {} times per unit.",
        range(&wakes_256),
        range(&sleeps_256),
    );
}

/// Extra time per unit over bare, and time inside `futex_wake` per unit.
/// Indicative: wall-clock, whatever else the machine is doing.
fn timed() {
    println!("INDICATIVE: wall-clock, not on an idle machine");
    println!(
        "{:<7} {:<6} {:>7} {:>10} {:>10} {:>10} {:>12}",
        "bytes", "lock", "threads", "bare ns", "extra ns", "wakes/u", "wake ns/u"
    );
    for (bytes, label) in FOOTPRINTS {
        for threads in THREADS {
            let units = threads as u64 * PER_THREAD;
            let bare = (0..RUNS)
                .map(|_| run_bare_on(Footprint::new(bytes), units).0.as_nanos() as f64)
                .fold(f64::INFINITY, f64::min)
                / units as f64;
            for kind in [LockKind::Mutex, LockKind::Futex] {
                // The best of the runs, and that run's counts.
                let (t, h): (f64, Handoffs) = (0..RUNS)
                    .map(|_| {
                        let start = Instant::now();
                        let h = handoffs(kind, bytes, threads, PER_THREAD);
                        (start.elapsed().as_nanos() as f64 / units as f64, h)
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .expect("at least one run");
                println!(
                    "{:<7} {:<6} {:>7} {:>10.0} {:>10.0} {:>10.3} {:>12.0}",
                    label,
                    kind.name(),
                    threads,
                    bare,
                    t - bare,
                    per(h.futex.wakes, h.units),
                    per(h.futex.wake_ns, h.units),
                );
            }
        }
    }
}

/// One run of std's mutex, for strace.
fn std_only(bytes: usize, threads: usize) {
    let h = handoffs(LockKind::Mutex, bytes, threads, PER_THREAD);
    println!(
        "std mutex, {bytes} B, {threads} threads: {} units, {} changes, {} sleeps",
        h.units, h.changes, h.sleeps
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => counts(),
        Some("--timed") => timed(),
        Some("--std") => {
            let arg = |i: usize| -> usize {
                args.get(i)
                    .and_then(|a| a.parse().ok())
                    .expect("--std <bytes> <threads>")
            };
            std_only(arg(1), arg(2));
        }
        Some(other) => panic!("unknown mode {other}: none, --timed, or --std <bytes> <threads>"),
    }
}
