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
//! `--pinned` counts the copy's calls with each thread pinned to its own
//! core, all in one core complex (`ccx1`) or split across the 2700X's two
//! (`ccx2`), at 2 and 4 threads: the waits that slept and were woken
//! beside the wakes, per placement, since a pinned std mutex cost about
//! three times as much split across complexes as within one. Counts, not
//! times, three runs each.
//!
//! Two other modes, neither for `results/`:
//!
//! - `--timed`: the same runs timed, with the time spent inside
//!   `futex_wake`, to set the counts beside the bench's extras. Then the
//!   pinned runs timed, std's mutex and the copy, with the copy's time per
//!   call inside `futex_wake`, inside `futex_wait`, and from a wake's send
//!   to the woken thread's return (wake-to-run). Wall-clock, so indicative
//!   only on a busy machine.
//! - `--std <bytes> <threads>`: one run of std's mutex and nothing else,
//!   for `strace -f -c -e trace=futex` to count its futex calls directly.

use std::time::Instant;

use bough_experiments::rfd_0006_lock_vs_queue_cost::{
    Footprint, Handoffs, LockKind, PLACEMENTS, handoffs, handoffs_on, run_bare_on, run_bare_pinned,
};

const FOOTPRINTS: [(usize, &str); 2] = [(0, "0B"), (256 * 1024, "256KiB")];
const THREADS: [usize; 3] = [2, 4, 8];
const PER_THREAD: u64 = 20_000;
const RUNS: usize = 3;

/// Threads in the pinned runs: at most 4 fit one CCX a core each.
const PINNED_THREADS: [usize; 2] = [2, 4];

fn per(n: u64, units: u64) -> f64 {
    n as f64 / units as f64
}

/// Nanoseconds per call, or `-` with no calls.
fn per_call(ns: u64, calls: u64) -> String {
    if calls == 0 {
        "-".to_string()
    } else {
        format!("{:.0}", ns as f64 / calls as f64)
    }
}

fn range(v: &[f64]) -> String {
    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = v.iter().copied().fold(0.0, f64::max);
    format!("{lo:.2}–{hi:.2}")
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

/// The copy's futex calls per unit, pinned in each placement.
fn pinned() {
    println!(
        "Futex calls per unit, the copy of std's mutex pinned a thread per core \
         ({PER_THREAD} units per thread, {RUNS} runs each)"
    );
    println!();
    println!(
        "{:<7} {:<5} {:>7} {:>4} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "bytes", "where", "threads", "run", "changes", "sleeps", "waits", "slept", "wakes", "woken"
    );
    // For the verdict: wakes per unit at 256 KiB, per placement.
    let mut wakes_256: [Vec<f64>; 2] = Default::default();
    for (bytes, label) in FOOTPRINTS {
        for (i, (placement, cpus)) in PLACEMENTS.into_iter().enumerate() {
            for threads in PINNED_THREADS {
                for run in 1..=RUNS {
                    let h = handoffs_on(
                        LockKind::Futex,
                        bytes,
                        threads,
                        &cpus[..threads],
                        PER_THREAD,
                    );
                    let u = h.units;
                    println!(
                        "{:<7} {:<5} {:>7} {:>4} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3}",
                        label,
                        placement,
                        threads,
                        run,
                        per(h.changes, u),
                        per(h.sleeps, u),
                        per(h.futex.waits, u),
                        per(h.futex.slept, u),
                        per(h.futex.wakes, u),
                        per(h.futex.woken, u),
                    );
                    if bytes > 0 {
                        wakes_256[i].push(per(h.futex.wakes, u));
                    }
                }
            }
        }
    }
    println!();
    println!("Per unit: every column is a count divided by the units run.");
    println!("where: ccx1 pins threads to CPUs 0, 2, 4, 6 (one core complex); ccx2 to");
    println!("0, 8, 2, 10 (alternating between the two). changes: the lock went to another");
    println!("thread. sleeps: voluntary context switches of the workers. waits: futex_wait");
    println!("calls; slept: those that slept and were woken. wakes: futex_wake calls;");
    println!("woken: wakes that woke a thread.");
    println!();
    println!(
        "Verdict: at 256 KiB, the copy made {} futex_wake calls per unit within one core \
         complex and {} split across two, over 2 and 4 threads.",
        range(&wakes_256[0]),
        range(&wakes_256[1]),
    );
}

/// The pinned runs timed: extra per unit over bare pinned to the first CPU,
/// for std's mutex and the copy, and the copy's time per futex call.
fn timed_pinned() {
    println!();
    println!("INDICATIVE: pinned, wall-clock, not on an idle machine; ns per call for the copy");
    println!(
        "{:<7} {:<5} {:<6} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "bytes",
        "where",
        "lock",
        "threads",
        "bare ns",
        "extra ns",
        "wakes/u",
        "wake",
        "slept/u",
        "wait",
        "to-run"
    );
    for (bytes, label) in FOOTPRINTS {
        for (placement, cpus) in PLACEMENTS {
            for threads in PINNED_THREADS {
                let cpus = &cpus[..threads];
                let units = threads as u64 * PER_THREAD;
                let bare = (0..RUNS)
                    .map(|_| {
                        let g = Footprint::new(bytes);
                        run_bare_pinned(g, cpus[0], units).0.as_nanos() as f64
                    })
                    .fold(f64::INFINITY, f64::min)
                    / units as f64;
                for kind in [LockKind::Mutex, LockKind::Futex] {
                    let (t, h): (f64, Handoffs) = (0..RUNS)
                        .map(|_| {
                            let start = Instant::now();
                            let h = handoffs_on(kind, bytes, threads, cpus, PER_THREAD);
                            (start.elapsed().as_nanos() as f64 / units as f64, h)
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .expect("at least one run");
                    let f = h.futex;
                    let copy = matches!(kind, LockKind::Futex);
                    let count = |n: u64| {
                        if copy {
                            format!("{:.3}", per(n, h.units))
                        } else {
                            "-".to_string()
                        }
                    };
                    println!(
                        "{:<7} {:<5} {:<6} {:>7} {:>8.0} {:>8.0} {:>8} {:>8} {:>8} {:>8} {:>8}",
                        label,
                        placement,
                        kind.name(),
                        threads,
                        bare,
                        t - bare,
                        count(f.wakes),
                        per_call(f.wake_ns, f.wakes),
                        count(f.slept),
                        per_call(f.wait_ns, f.waits),
                        per_call(f.wake_to_run_ns, f.slept),
                    );
                }
            }
        }
    }
    println!();
    println!("wake: ns inside each futex_wake. wait: ns inside each futex_wait, asleep");
    println!("behind the holder included. to-run: ns from a wake's send to the woken");
    println!("thread's return from futex_wait, over waits that slept (exact at 2 threads,");
    println!("reads low at 4). Timed run: the best of {RUNS}, with its own counts.");
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
        Some("--timed") => {
            timed();
            timed_pinned();
        }
        Some("--pinned") => pinned(),
        Some("--std") => {
            let arg = |i: usize| -> usize {
                args.get(i)
                    .and_then(|a| a.parse().ok())
                    .expect("--std <bytes> <threads>")
            };
            std_only(arg(1), arg(2));
        }
        Some(other) => {
            panic!("unknown mode {other}: none, --pinned, --timed, or --std <bytes> <threads>")
        }
    }
}
