//! Does "threading costs overhead" hold in Rust for the alternatives RFD 6
//! rejects, and how much?
//!
//! RFD 6 keeps the engine single-threaded: one driver owns the runtime, and
//! every call through an `Io` or a `RemoteIo` queues for the driver's next
//! pump. The alternative it rejects is running calls at once from whatever
//! thread makes them, which in Rust means the runtime behind a mutex. The
//! no-std handoff gave "threading and coordination overhead" as a reason,
//! with no source. The one measurement the literature review found,
//! Drechsler et al. 2018 (p. 21), says an uncontended global lock around
//! each propagation is negligible, and that a contended one makes the same
//! workload "noticeably slower" at two threads; their updates took about
//! 6.5 µs each on the JVM. Bough's units are much cheaper (F66: 528 ns), so
//! the lock's share of a unit could be larger here.
//!
//! This module is one unit of work and the ways to run it:
//!
//! - `bare`, the baseline: the owner thread calls the propagation directly.
//! - `locked`: the propagation behind a `std::sync::Mutex`, locked once per
//!   unit, uncontended on one thread and contended by 2, 4 and 8 threads
//!   that each submit units back to back.
//! - `queued`: RFD 6's design. A call boxes the unit as a `Send` closure
//!   (so the edge allocates only on the sending side, F80), pushes it on
//!   an inbox, and wakes the driver unless a call already has since the
//!   last pump began. The driver's pump takes the count queued when it
//!   begins (F77: units queued during a pump wait for the next one) and
//!   pops them one at a time, each under the lock and each run after the
//!   lock is released, as the engine spike's inbox does.
//!
//! The unit is a fixed 50-node propagation: every node recomputed in one
//! flat loop, in index order, from two earlier nodes, with a dependent
//! chain of multiply-xors as its own work. `ROUNDS` sets that work; it was
//! calibrated once on the Ryzen 7 2700X to about 500 ns per propagation
//! and is fixed here (the ignored `calibration` test prints it again). A
//! propagation is a pure function of its input, and each variant adds its
//! results into a sum, so every variant over the same inputs must end with
//! the same sum, whatever order the threads ran them in.
//!
//! The inbox is a `Mutex<VecDeque>`, not `std::sync::mpsc`, because that is
//! what the spike's `RemoteIo` inbox is and what RFD 6 needs from one: the
//! pump reads the queue's length and resets the wake flag under the same
//! lock a call stamps and wakes under, and the collector walks the waiting
//! registrations for roots, neither of which a channel allows. It is also
//! std's mutex, which F78 found unfair; that is the lock both designs
//! would ship on `std`.
//!
//! What can be counted where: the single-threaded runs (bare, uncontended
//! lock, and queue then pump on one thread) are deterministic and go
//! through the instruction counts. The contended runs involve several
//! threads, where an instruction count says nothing about waiting, so they
//! are wall-clock only: throughput (`run_*`, timed from a barrier) and the
//! spread of waits (`lock_spread`, `queue_spread`), since an unfair lock
//! shows up in the tail, not the mean.

use std::collections::VecDeque;
use std::sync::{Arc, Barrier, Mutex, MutexGuard, PoisonError};
use std::task::{Wake, Waker};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

/// Nodes in one propagation.
pub const NODES: usize = 50;

/// Multiply-xor rounds per node: the node's own work. Calibrated once to
/// put a propagation near 500 ns on the Ryzen 7 2700X (about 10 ns a
/// node): 6 rounds gave 497 ns, 7 gave 564 ns, on 2026-09-28, not idle.
/// See the `calibration` test.
pub const ROUNDS: u32 = 6;

/// Units per burst in the single-threaded runs, and the queue's burst.
pub const BURST: u64 = 64;

#[derive(Clone, Copy)]
struct Node {
    value: u64,
    /// The propagation that last wrote the node, as an engine's stamp.
    stamp: u64,
    deps: [u32; 2],
}

/// The runtime stand-in: 50 nodes, 24 bytes each, and a running sum of
/// every propagation's result, which is what a listener would see.
#[derive(Clone)]
pub struct Graph {
    nodes: Vec<Node>,
    stamp: u64,
    sink: u64,
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    pub fn new() -> Graph {
        let nodes = (0..NODES as u32)
            .map(|i| Node {
                value: 0,
                stamp: 0,
                // The previous node and one further back, so the loop
                // reads both near and far.
                deps: if i == 0 {
                    [0, 0]
                } else {
                    [i - 1, (i * 37 + 11) % i]
                },
            })
            .collect();
        Graph {
            nodes,
            stamp: 0,
            sink: 0,
        }
    }

    /// One unit: set the input node, recompute every other node in order,
    /// and add the last node's value to the sum.
    #[inline(never)]
    pub fn propagate(&mut self, input: u64) -> u64 {
        self.stamp += 1;
        let stamp = self.stamp;
        self.nodes[0].value = input;
        self.nodes[0].stamp = stamp;
        for i in 1..NODES {
            let [a, b] = self.nodes[i].deps;
            let mut v = self.nodes[a as usize].value ^ self.nodes[b as usize].value.rotate_left(17);
            for _ in 0..ROUNDS {
                v = (v ^ (v >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            }
            self.nodes[i] = Node {
                value: v,
                stamp,
                ..self.nodes[i]
            };
        }
        let out = self.nodes[NODES - 1].value;
        self.sink = self.sink.wrapping_add(out);
        out
    }

    /// The sum of every result so far.
    pub fn sink(&self) -> u64 {
        self.sink
    }

    /// How many units have run.
    pub fn units(&self) -> u64 {
        self.stamp
    }
}

/// The sum the inputs `0..units` give, run bare.
pub fn expected(units: u64) -> u64 {
    let mut g = Graph::new();
    bare(&mut g, 0, units);
    g.sink()
}

/// Units `first..first + n` on the owner thread.
pub fn bare(g: &mut Graph, first: u64, n: u64) -> u64 {
    for k in first..first + n {
        g.propagate(k);
    }
    g.sink()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // No unit panics, but a poisoned lock would still hold a whole graph.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Units `first..first + n`, each under its own lock.
pub fn locked(m: &Mutex<Graph>, first: u64, n: u64) -> u64 {
    for k in first..first + n {
        lock(m).propagate(k);
    }
    lock(m).sink()
}

/// A queued unit, as the spike's: a boxed closure the driver runs.
pub type Unit = Box<dyn FnOnce(&mut Graph) + Send>;

/// The `RemoteIo`'s inbox: calls, the driver's waker, and whether a call
/// has woken it since the last pump began, all under one lock.
pub struct Inbox {
    state: Mutex<InboxState>,
}

struct InboxState {
    calls: VecDeque<Unit>,
    waker: Option<Waker>,
    woken: bool,
}

impl Inbox {
    pub fn new(waker: Waker) -> Inbox {
        Inbox {
            state: Mutex::new(InboxState {
                calls: VecDeque::new(),
                waker: Some(waker),
                woken: false,
            }),
        }
    }

    /// A `send(input, k)` through the handle: box the unit on the caller,
    /// queue it, and wake the driver after the lock is released, unless a
    /// call has since the last pump began.
    pub fn send(&self, k: u64) {
        let unit: Unit = Box::new(move |g: &mut Graph| {
            g.propagate(k);
        });
        let waker = {
            let mut q = lock(&self.state);
            q.calls.push_back(unit);
            if q.woken {
                None
            } else {
                q.woken = true;
                q.waker.clone()
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// A pump begins: the next call wakes the driver again, and the pump
    /// runs only the calls queued before now.
    fn begin_pump(&self) -> usize {
        let mut q = lock(&self.state);
        q.woken = false;
        q.calls.len()
    }

    /// The oldest call, taken under the lock and run after it.
    fn pop(&self) -> Option<Unit> {
        lock(&self.state).calls.pop_front()
    }
}

/// The driver's pump: runs the units queued when it began, one lock per
/// unit. Returns how many ran.
pub fn pump(inbox: &Inbox, g: &mut Graph) -> usize {
    let limit = inbox.begin_pump();
    for _ in 0..limit {
        let unit = inbox.pop().expect("a unit counted at the pump's start");
        unit(g);
    }
    limit
}

/// Units `first..first + n` sent through the inbox in bursts of `burst`,
/// the driver pumping after each burst, all on one thread.
pub fn queued(inbox: &Inbox, g: &mut Graph, first: u64, n: u64, burst: u64) -> u64 {
    let mut k = first;
    while k < first + n {
        let end = (k + burst).min(first + n);
        for j in k..end {
            inbox.send(j);
        }
        pump(inbox, g);
        k = end;
    }
    g.sink()
}

/// A thread driver's waker: unparks the driver.
struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// A waker that unparks the calling thread.
pub fn thread_waker() -> Waker {
    Waker::from(Arc::new(Unpark(thread::current())))
}

/// Everything the single-threaded runs need, warmed: the inbox has grown
/// to a burst once, so a counted run doesn't include its growth.
pub struct Single {
    pub graph: Graph,
    pub mutex: Mutex<Graph>,
    pub inbox: Inbox,
}

impl Default for Single {
    fn default() -> Self {
        Self::new()
    }
}

impl Single {
    pub fn new() -> Single {
        let mut s = Single {
            graph: Graph::new(),
            mutex: Mutex::new(Graph::new()),
            inbox: Inbox::new(thread_waker()),
        };
        let mut warm = Graph::new();
        queued(&s.inbox, &mut warm, 0, BURST, BURST);
        bare(&mut s.graph, 0, 1);
        locked(&s.mutex, 0, 1);
        s
    }
}

/// Thread `t` of `threads`' share of units `0..units`.
fn share(t: usize, threads: usize, units: u64) -> std::ops::Range<u64> {
    let (t, n) = (t as u64, threads as u64);
    t * units / n..(t + 1) * units / n
}

/// `units` units run bare on this thread, timed. The contended runs'
/// baseline: the engine runs units one at a time whoever calls, so the
/// best any design can do is this.
pub fn run_bare(units: u64) -> (Duration, u64) {
    let mut g = Graph::new();
    let start = Instant::now();
    let sink = bare(&mut g, 0, units);
    (start.elapsed(), sink)
}

/// `threads` threads each lock-and-run their share of `units`, back to
/// back. Timed from the barrier that releases them to the last one's end.
pub fn run_locked(threads: usize, units: u64) -> (Duration, u64) {
    let m = Mutex::new(Graph::new());
    let barrier = Barrier::new(threads + 1);
    let start = thread::scope(|s| {
        for t in 0..threads {
            let (m, barrier) = (&m, &barrier);
            s.spawn(move || {
                let range = share(t, threads, units);
                barrier.wait();
                for k in range {
                    lock(m).propagate(k);
                }
            });
        }
        barrier.wait();
        Instant::now()
    });
    // The scope has joined every thread by here.
    let elapsed = start.elapsed();
    let sink = m
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .sink();
    (elapsed, sink)
}

/// `producers` threads each send their share of `units` through the inbox
/// as fast as they can, and this thread drives: pumps while there is
/// anything queued, parks when there isn't. Timed from the barrier to the
/// last unit run.
pub fn run_queued(producers: usize, units: u64) -> (Duration, u64) {
    let inbox = Inbox::new(thread_waker());
    let mut g = Graph::new();
    let barrier = Barrier::new(producers + 1);
    let elapsed = thread::scope(|s| {
        for t in 0..producers {
            let (inbox, barrier) = (&inbox, &barrier);
            s.spawn(move || {
                let range = share(t, producers, units);
                barrier.wait();
                for k in range {
                    inbox.send(k);
                }
            });
        }
        barrier.wait();
        let start = Instant::now();
        let mut done = 0;
        while done < units {
            let ran = pump(&inbox, &mut g) as u64;
            done += ran;
            if ran == 0 {
                thread::park();
            }
        }
        start.elapsed()
    });
    (elapsed, g.sink())
}

/// A distribution of waits, in nanoseconds.
#[derive(Clone, Copy, Debug)]
pub struct Spread {
    pub samples: usize,
    pub p50: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

impl Spread {
    fn of(mut ns: Vec<u64>) -> Spread {
        ns.sort_unstable();
        let at = |q: f64| ns[((ns.len() - 1) as f64 * q) as usize];
        Spread {
            samples: ns.len(),
            p50: at(0.5),
            p99: at(0.99),
            p999: at(0.999),
            max: *ns.last().expect("at least one sample"),
        }
    }
}

fn nanos(d: Duration) -> u64 {
    d.as_nanos() as u64
}

/// The lock contended, looked at from the callers: each call's wait for
/// the lock, and the longest run of units one thread got in a row, which
/// is what an unfair lock shows: with every thread hammering, the others
/// wait through all of it.
#[derive(Clone, Copy, Debug)]
pub struct LockSpread {
    pub wait: Spread,
    pub longest_streak: u64,
}

pub fn lock_spread(threads: usize, per_thread: u64) -> LockSpread {
    // The owner of the last unit and the current streak ride in the lock
    // with the graph; a few instructions beside a 500 ns unit.
    let m = Mutex::new((Graph::new(), usize::MAX, 0u64, 0u64));
    let barrier = Barrier::new(threads);
    let waits: Vec<Vec<u64>> = thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (m, barrier) = (&m, &barrier);
                s.spawn(move || {
                    let mut waits = Vec::with_capacity(per_thread as usize);
                    barrier.wait();
                    for k in t as u64 * per_thread..(t as u64 + 1) * per_thread {
                        let asked = Instant::now();
                        let mut state = lock(m);
                        waits.push(nanos(asked.elapsed()));
                        let (g, owner, streak, longest) = &mut *state;
                        g.propagate(k);
                        *streak = if *owner == t { *streak + 1 } else { 1 };
                        *owner = t;
                        *longest = (*longest).max(*streak);
                    }
                    waits
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("no unit panics"))
            .collect()
    });
    let (g, _, _, longest) = m.into_inner().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(g.units(), threads as u64 * per_thread);
    LockSpread {
        wait: Spread::of(waits.concat()),
        longest_streak: longest,
    }
}

/// The queue under the same load, from both ends: how long a producer's
/// call blocks (the lock, the box, the wake), and how long each of the
/// driver's pops takes while producers hammer the same lock, which is
/// F78's question about a drain. A unit's time from call to run isn't
/// reported: with producers faster than the driver it measures the burst's
/// length, not the queue.
#[derive(Clone, Copy, Debug)]
pub struct QueueSpread {
    pub send: Spread,
    pub pop: Spread,
    pub longest_pump: usize,
}

pub fn queue_spread(producers: usize, per_thread: u64) -> QueueSpread {
    let inbox = Inbox::new(thread_waker());
    let mut g = Graph::new();
    let barrier = Barrier::new(producers + 1);
    let units = producers as u64 * per_thread;
    let (sends, pops, longest_pump) = thread::scope(|s| {
        let handles: Vec<_> = (0..producers)
            .map(|t| {
                let (inbox, barrier) = (&inbox, &barrier);
                s.spawn(move || {
                    let mut sends = Vec::with_capacity(per_thread as usize);
                    barrier.wait();
                    for k in t as u64 * per_thread..(t as u64 + 1) * per_thread {
                        let called = Instant::now();
                        inbox.send(k);
                        sends.push(nanos(called.elapsed()));
                    }
                    sends
                })
            })
            .collect();
        barrier.wait();
        let mut pops = Vec::with_capacity(units as usize);
        let mut longest_pump = 0;
        let mut done = 0;
        while done < units {
            // The pump, written out so each pop is timed.
            let limit = inbox.begin_pump();
            for _ in 0..limit {
                let asked = Instant::now();
                let unit = inbox.pop().expect("a unit counted at the pump's start");
                pops.push(nanos(asked.elapsed()));
                unit(&mut g);
            }
            longest_pump = longest_pump.max(limit);
            done += limit as u64;
            if limit == 0 {
                thread::park();
            }
        }
        let sends: Vec<u64> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("no unit panics"))
            .collect();
        (sends, pops, longest_pump)
    });
    assert_eq!(g.units(), units);
    QueueSpread {
        send: Spread::of(sends),
        pop: Spread::of(pops),
        longest_pump,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant runs every unit exactly once: same sum as bare.
    #[test]
    fn variants_agree() {
        let want = expected(1_000);
        let s = Single::new();
        let mut g = Graph::new();
        assert_eq!(bare(&mut g, 0, 1_000), want);
        let m = Mutex::new(Graph::new());
        assert_eq!(locked(&m, 0, 1_000), want);
        for burst in [1, BURST] {
            let mut g = Graph::new();
            assert_eq!(queued(&s.inbox, &mut g, 0, 1_000, burst), want);
            assert_eq!(g.units(), 1_000);
        }
        assert_eq!(run_bare(1_000).1, want);
        for threads in [1, 2, 4, 8] {
            assert_eq!(run_locked(threads, 1_000).1, want, "lock x{threads}");
            assert_eq!(run_queued(threads, 1_000).1, want, "queue x{threads}");
        }
    }

    #[test]
    fn spreads_run() {
        let l = lock_spread(4, 500);
        assert_eq!(l.wait.samples, 2_000);
        assert!(l.longest_streak >= 1);
        let q = queue_spread(4, 500);
        assert_eq!(q.send.samples, 2_000);
        assert_eq!(q.pop.samples, 2_000);
    }

    /// Prints the time of one propagation, to check `ROUNDS` against the
    /// 500 ns target. Wall-clock, so ignored:
    /// `cargo test --release --lib rfd_0006 -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn calibration() {
        let units = 2_000_000;
        run_bare(units / 10);
        let best = (0..5)
            .map(|_| run_bare(units).0.as_nanos() as f64 / units as f64)
            .fold(f64::INFINITY, f64::min);
        println!("ROUNDS = {ROUNDS}: {best:.0} ns per propagation");
    }
}
