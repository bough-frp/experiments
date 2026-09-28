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
//!
//! ## Where a contended lock's extra time goes
//!
//! The first run found the contended lock costing 1.6–1.9× bare per unit
//! and couldn't say why. Two candidates: the graph's state migrating
//! between cores' caches with the lock (every hand-off to another thread
//! means that thread's first touch of each line the last owner wrote
//! misses to the other core), or the lock's hand-off itself (the futex
//! wake of a parked waiter, and the lock word's own line). A single-owner
//! queue avoids the first by design; it pays its own hand-off per call.
//!
//! [`Footprint`] separates them. It is a second runtime stand-in whose
//! work per unit is fixed (the same 50-node, `ROUNDS`-round chain as
//! [`Graph`], computed in a stack array that stays in the running core's
//! cache) and whose state is a parameter: `bytes` of heap, every 64-byte
//! line of it read-modified-written once per unit. So between footprints
//! only the lines a unit dirties change, and with them what a hand-off
//! must migrate. One word per line is the least that touches a line, so
//! the unit's own cost still grows with the footprint (4,096 extra
//! read-modify-writes at 256 KiB); each footprint has its own bare
//! baseline, and what's compared is the extra over it. The lines are
//! visited in a fixed stride-permuted order, as a graph walk isn't a
//! sequential stream the prefetcher could run ahead of from another
//! core's cache.
//!
//! At 0 bytes the unit touches no state but the struct beside the lock
//! word (its stamp and sum), so a lock's extra there is its hand-off
//! alone. The contended runs take two locks and the queue over every
//! footprint: `std::sync::Mutex` (spins briefly, then parks on a futex)
//! and [`Ticket`] (a fair spin lock: never parks, so no futex wake, and
//! hands off every unit, so the state migrates every unit). The ticket
//! lock's extra over bare, against the footprint, is the migration cost
//! per hand-off, its slope, on top of a bare hand-off, its intercept. The
//! mutex's extra is its share of units that were hand-offs times (a
//! futex wake plus that migration): [`handoffs`] counts the share, since
//! an unfair lock that lets one thread run a thousand units in a row
//! migrates the state once per thousand units, not once per unit.
//!
//! ## Futex calls, and which cores the state crosses
//!
//! The footprint sweep found migration small and the mutex's extra mostly
//! something else: at 256 KiB it almost never changes threads and still
//! costs 1.1–1.4 µs a unit. The guess was a `futex_wake` on each unlock
//! while other threads sleep. Std's mutex is private, so it can't be asked
//! how often it calls the kernel; two things can be counted instead.
//! [`FutexMutex`] is std's Linux futex mutex copied line for line (the same
//! three states, the same 100-load spin, the same syscalls) with a count of
//! each `futex_wait` and `futex_wake` it makes and how many threads each
//! wake woke. And for std's own mutex, each worker's voluntary context
//! switches from `getrusage(RUSAGE_THREAD)`: a thread that sleeps on a
//! futex switches out voluntarily, so this counts the sleeps a wake then
//! has to end, without touching the lock. [`handoffs`] reports both.
//!
//! The Ryzen 7 2700X's 8 cores are two core complexes (CCXs) of 4, each
//! with its own L3 (CPUs 0–7 and 8–15 here, SMT siblings adjacent). A line
//! moving between cores in one CCX is an L3 hit; between CCXs it crosses
//! the Infinity Fabric. The unpinned runs let the scheduler place threads
//! anywhere, so their slope mixes the two. [`PLACEMENTS`] pins each thread
//! to its own physical core, all in one CCX (`ccx1`) or alternating
//! between the two (`ccx2`), through `libc::sched_setaffinity`.

use std::cell::{Cell, UnsafeCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
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

/// A runtime stand-in the variants can run: one unit is a pure function
/// of its input, added into a running sum.
pub trait State: Send {
    fn propagate(&mut self, input: u64) -> u64;
    /// The sum of every result so far.
    fn sink(&self) -> u64;
    /// How many units have run.
    fn units(&self) -> u64;
}

impl State for Graph {
    fn propagate(&mut self, input: u64) -> u64 {
        Graph::propagate(self, input)
    }

    fn sink(&self) -> u64 {
        Graph::sink(self)
    }

    fn units(&self) -> u64 {
        Graph::units(self)
    }
}

/// The graph's dependencies, as [`Graph::new`] sets them, for
/// [`Footprint`]'s chain: a table, so the chain doesn't divide per node.
const DEPS: [[u8; 2]; NODES] = {
    let mut deps = [[0; 2]; NODES];
    let mut i = 1;
    while i < NODES {
        deps[i] = [(i - 1) as u8, ((i * 37 + 11) % i) as u8];
        i += 1;
    }
    deps
};

/// The 50-node chain on the stack: the same work as [`Graph::propagate`],
/// but the values are the running thread's own, so they never migrate.
#[inline(never)]
pub fn chain(input: u64) -> u64 {
    let mut values = [0u64; NODES];
    values[0] = input;
    for i in 1..NODES {
        let [a, b] = DEPS[i];
        let mut v = values[a as usize] ^ values[b as usize].rotate_left(17);
        for _ in 0..ROUNDS {
            v = (v ^ (v >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        }
        values[i] = v;
    }
    values[NODES - 1]
}

/// One cache line of [`Footprint`]'s state. Only word 0 is touched.
#[derive(Clone, Copy)]
#[repr(align(64))]
struct Line([u64; 8]);

/// The runtime stand-in with a footprint: the chain's fixed work, then
/// every line of `bytes` of state read-modified-written once.
pub struct Footprint {
    lines: Vec<Line>,
    /// The visiting order's step: coprime with the line count, so one
    /// pass visits every line once, not in address order.
    step: usize,
    stamp: u64,
    sink: u64,
}

/// The footprints the benches take: none (the hand-off alone), one line,
/// the 50-node graph's 1.2 KB (19 lines, as [`Graph`]'s 50 × 24 B), and
/// two beyond it, 16 KiB (in L1, 32 KiB on Zen+) and 256 KiB (in L2,
/// 512 KiB).
pub const FOOTPRINTS: [(usize, &str); 5] = [
    (0, "0B"),
    (64, "64B"),
    (1216, "1216B"),
    (16 * 1024, "16KiB"),
    (256 * 1024, "256KiB"),
];

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl Footprint {
    /// `bytes`, a multiple of 64, of state.
    pub fn new(bytes: usize) -> Footprint {
        assert_eq!(bytes % 64, 0, "a footprint is whole lines");
        let n = bytes / 64;
        // A non-zero word per line, so the allocation is written now and
        // not a lazily zeroed mapping whose first touch page-faults inside
        // a timed run.
        let lines = vec![Line([0, 0, 0, 0, 0, 0, 0, !0]); n];
        // About 5/8 of the way round, the first step from there coprime
        // with `n`.
        let mut step = (n * 5 / 8).max(1);
        while n > 1 && gcd(step, n) != 1 {
            step += 1;
        }
        Footprint {
            lines,
            step,
            stamp: 0,
            sink: 0,
        }
    }

    /// Whether every line has seen every unit: each line's word is a sum
    /// of results, in whatever order they came, so it equals the sink.
    pub fn consistent(&self) -> bool {
        self.lines.iter().all(|l| l.0[0] == self.sink)
    }
}

impl State for Footprint {
    #[inline(never)]
    fn propagate(&mut self, input: u64) -> u64 {
        self.stamp += 1;
        let out = chain(input);
        let n = self.lines.len();
        let mut l = 0;
        for _ in 0..n {
            let w = &mut self.lines[l].0[0];
            *w = w.wrapping_add(out);
            l += self.step;
            if l >= n {
                l -= n;
            }
        }
        self.sink = self.sink.wrapping_add(out);
        out
    }

    fn sink(&self) -> u64 {
        self.sink
    }

    fn units(&self) -> u64 {
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
pub fn bare<S: State>(g: &mut S, first: u64, n: u64) -> u64 {
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
pub fn locked<S: State>(m: &Mutex<S>, first: u64, n: u64) -> u64 {
    for k in first..first + n {
        lock(m).propagate(k);
    }
    lock(m).sink()
}

/// Units `first..first + n`, each under its own ticket lock.
pub fn ticketed<S: State>(m: &Ticket<S>, first: u64, n: u64) -> u64 {
    for k in first..first + n {
        m.with(|g| g.propagate(k));
    }
    m.with(|g| g.sink())
}

/// A lock the contended runs can take: std's mutex or [`Ticket`].
pub trait Lock<T>: Sync {
    fn new(value: T) -> Self;
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R;
    fn into_inner(self) -> T;
}

impl<T: Send> Lock<T> for Mutex<T> {
    fn new(value: T) -> Self {
        Mutex::new(value)
    }

    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut lock(self))
    }

    fn into_inner(self) -> T {
        Mutex::into_inner(self).unwrap_or_else(PoisonError::into_inner)
    }
}

/// A ticket lock: a spin lock that never parks, and is fair. A caller
/// takes the next ticket and spins until the lock serves it, so under
/// contention the lock goes to a different thread every unit, in turn.
/// Beside std's mutex, which spins a little and then sleeps on a futex, it
/// has no wake-ups, and every unit is a hand-off: its extra over bare is
/// the lock word's line and the state's lines crossing cores once per
/// unit. (A test-and-set spin lock was tried first and handed off to
/// another thread about once in 200 units: the releasing thread takes it
/// straight back, so it measured no migration at all.) It is only sound
/// for timing because no thread waits on it that isn't running: the
/// contended runs use at most 8 of the 16 hardware threads, and a ticket
/// holder descheduled by other load stalls every thread behind it.
pub struct Ticket<T> {
    next: AtomicU32,
    serving: AtomicU32,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reached only through `with`, by the one thread
// whose ticket is being served, as a `Mutex` would.
unsafe impl<T: Send> Sync for Ticket<T> {}

impl<T: Send> Lock<T> for Ticket<T> {
    fn new(value: T) -> Self {
        Ticket {
            next: AtomicU32::new(0),
            serving: AtomicU32::new(0),
            value: UnsafeCell::new(value),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mine = self.next.fetch_add(1, Ordering::Relaxed);
        while self.serving.load(Ordering::Acquire) != mine {
            std::hint::spin_loop();
        }
        // SAFETY: only the holder of the served ticket gets here, and the
        // next ticket isn't served until the store below. No unit panics;
        // if one did, the lock would stay held, which only stops the run.
        let out = f(unsafe { &mut *self.value.get() });
        self.serving.store(mine.wrapping_add(1), Ordering::Release);
        out
    }

    fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

/// A copy of std's futex mutex on Linux (`library/std/src/sys/sync/mutex/
/// futex.rs` and `sys/pal/unix/futex.rs` at 1.98.1), counting the calls it
/// makes into the kernel in the calling thread's [`FutexCalls`]. Only the
/// counting is added, and it is a thread-local add beside a syscall.
pub struct FutexMutex<T> {
    /// 0 unlocked, 1 locked, 2 locked with waiters (maybe).
    state: AtomicU32,
    value: UnsafeCell<T>,
}

const UNLOCKED: u32 = 0;
const LOCKED: u32 = 1;
const CONTENDED: u32 = 2;

// SAFETY: as `Ticket`: the value is reached only through `with`, by the
// thread that holds the lock.
unsafe impl<T: Send> Sync for FutexMutex<T> {}

/// A thread's calls into the kernel through [`FutexMutex`].
#[derive(Clone, Copy, Debug, Default)]
pub struct FutexCalls {
    /// `futex_wait` syscalls, whether they slept or returned at once.
    pub waits: u64,
    /// `futex_wake` syscalls: an unlock that found the lock contended.
    pub wakes: u64,
    /// Threads those wakes woke (0 or 1 each).
    pub woken: u64,
    /// Time inside those `futex_wake` calls, which the unlocking thread
    /// pays before it can take the lock again. A timing: indicative only.
    pub wake_ns: u64,
}

impl std::ops::Add for FutexCalls {
    type Output = FutexCalls;
    fn add(self, o: FutexCalls) -> FutexCalls {
        FutexCalls {
            waits: self.waits + o.waits,
            wakes: self.wakes + o.wakes,
            woken: self.woken + o.woken,
            wake_ns: self.wake_ns + o.wake_ns,
        }
    }
}

thread_local! {
    static CALLS: Cell<FutexCalls> = const {
        Cell::new(FutexCalls { waits: 0, wakes: 0, woken: 0, wake_ns: 0 })
    };
}

/// This thread's [`FutexMutex`] calls so far.
pub fn futex_calls() -> FutexCalls {
    CALLS.with(Cell::get)
}

impl<T> FutexMutex<T> {
    fn lock(&self) {
        if self
            .state
            .compare_exchange(UNLOCKED, LOCKED, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            self.lock_contended();
        }
    }

    #[cold]
    fn lock_contended(&self) {
        let mut state = self.spin();
        if state == UNLOCKED {
            match self.state.compare_exchange(
                UNLOCKED,
                LOCKED,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(s) => state = s,
            }
        }
        loop {
            if state != CONTENDED && self.state.swap(CONTENDED, Ordering::Acquire) == UNLOCKED {
                return;
            }
            self.wait();
            state = self.spin();
        }
    }

    fn spin(&self) -> u32 {
        let mut spin = 100;
        loop {
            let state = self.state.load(Ordering::Relaxed);
            if state != LOCKED || spin == 0 {
                return state;
            }
            std::hint::spin_loop();
            spin -= 1;
        }
    }

    /// Std's `futex_wait(&state, CONTENDED, None)`: no timeout, retried on
    /// EINTR, skipped if the state already moved.
    fn wait(&self) {
        loop {
            if self.state.load(Ordering::Relaxed) != CONTENDED {
                return;
            }
            CALLS.with(|c| {
                let mut v = c.get();
                v.waits += 1;
                c.set(v)
            });
            // SAFETY: a futex wait on our own live atomic, as std's.
            let r = unsafe {
                libc::syscall(
                    libc::SYS_futex,
                    self.state.as_ptr(),
                    libc::FUTEX_WAIT_BITSET | libc::FUTEX_PRIVATE_FLAG,
                    CONTENDED,
                    std::ptr::null::<libc::timespec>(),
                    std::ptr::null::<u32>(),
                    !0u32,
                )
            };
            let interrupted =
                r < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR);
            if !interrupted {
                return;
            }
        }
    }

    fn unlock(&self) {
        if self.state.swap(UNLOCKED, Ordering::Release) == CONTENDED {
            self.wake();
        }
    }

    #[cold]
    fn wake(&self) {
        let started = Instant::now();
        // SAFETY: a futex wake of one waiter on our own atomic, as std's.
        let r = unsafe {
            libc::syscall(
                libc::SYS_futex,
                self.state.as_ptr(),
                libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
                1,
            )
        };
        CALLS.with(|c| {
            let mut v = c.get();
            v.wakes += 1;
            v.woken += (r > 0) as u64;
            v.wake_ns += nanos(started.elapsed());
            c.set(v)
        });
    }
}

impl<T: Send> Lock<T> for FutexMutex<T> {
    fn new(value: T) -> Self {
        FutexMutex {
            state: AtomicU32::new(UNLOCKED),
            value: UnsafeCell::new(value),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        self.lock();
        // SAFETY: we hold the lock until the unlock below. No unit panics;
        // if one did, the lock would stay held, which only stops the run.
        let out = f(unsafe { &mut *self.value.get() });
        self.unlock();
        out
    }

    fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

/// This thread's voluntary context switches so far: each is a sleep, on a
/// futex or anything else that blocks.
pub fn voluntary_switches() -> u64 {
    // SAFETY: getrusage fills the struct it is given.
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        let r = libc::getrusage(libc::RUSAGE_THREAD, &mut u);
        assert_eq!(r, 0, "getrusage(RUSAGE_THREAD)");
        u.ru_nvcsw as u64
    }
}

/// Pins the calling thread to one CPU.
pub fn pin_to(cpu: usize) {
    // SAFETY: a zeroed `cpu_set_t` is an empty set, and `CPU_SET` stays
    // inside it for any CPU below 1024.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        let r = libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &set);
        assert_eq!(r, 0, "sched_setaffinity to CPU {cpu}");
    }
}

/// Where the pinned runs put their threads, one per physical core, thread
/// `t` on `cpus[t]`: `ccx1` all in the first CCX (CPUs 0–7), `ccx2`
/// alternating between it and the second (8–15), so 2 threads are one in
/// each and 4 are two in each. SMT siblings are adjacent CPUs on this
/// machine (`lscpu -e`), so even CPUs are distinct cores.
pub const PLACEMENTS: [(&str, [usize; 4]); 2] = [("ccx1", [0, 2, 4, 6]), ("ccx2", [0, 8, 2, 10])];

/// A queued unit, as the spike's: a boxed closure the driver runs.
pub type Unit<S = Graph> = Box<dyn FnOnce(&mut S) + Send>;

/// The `RemoteIo`'s inbox: calls, the driver's waker, and whether a call
/// has woken it since the last pump began, all under one lock.
pub struct Inbox<S = Graph> {
    state: Mutex<InboxState<S>>,
}

struct InboxState<S> {
    calls: VecDeque<Unit<S>>,
    waker: Option<Waker>,
    woken: bool,
}

impl<S: State> Inbox<S> {
    pub fn new(waker: Waker) -> Inbox<S> {
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
        let unit: Unit<S> = Box::new(move |g: &mut S| {
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
    fn pop(&self) -> Option<Unit<S>> {
        lock(&self.state).calls.pop_front()
    }
}

/// The driver's pump: runs the units queued when it began, one lock per
/// unit. Returns how many ran.
pub fn pump<S: State>(inbox: &Inbox<S>, g: &mut S) -> usize {
    let limit = inbox.begin_pump();
    for _ in 0..limit {
        let unit = inbox.pop().expect("a unit counted at the pump's start");
        unit(g);
    }
    limit
}

/// Units `first..first + n` sent through the inbox in bursts of `burst`,
/// the driver pumping after each burst, all on one thread.
pub fn queued<S: State>(inbox: &Inbox<S>, g: &mut S, first: u64, n: u64, burst: u64) -> u64 {
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
pub struct Single<S = Graph> {
    pub graph: S,
    pub mutex: Mutex<S>,
    pub ticket: Ticket<S>,
    pub inbox: Inbox<S>,
}

impl Default for Single {
    fn default() -> Self {
        Self::new()
    }
}

impl Single {
    pub fn new() -> Single {
        Single::of(Graph::new)
    }
}

impl<S: State> Single<S> {
    /// Each variant's own state from `make`, warmed.
    pub fn of(make: impl Fn() -> S) -> Single<S> {
        let mut s = Single {
            graph: make(),
            mutex: Mutex::new(make()),
            ticket: Lock::new(make()),
            inbox: Inbox::new(thread_waker()),
        };
        let mut warm = make();
        queued(&s.inbox, &mut warm, 0, BURST, BURST);
        bare(&mut s.graph, 0, 1);
        locked(&s.mutex, 0, 1);
        ticketed(&s.ticket, 0, 1);
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
    run_bare_on(Graph::new(), units)
}

/// [`run_bare`] on any state.
pub fn run_bare_on<S: State>(mut g: S, units: u64) -> (Duration, u64) {
    let start = Instant::now();
    let sink = bare(&mut g, 0, units);
    (start.elapsed(), sink)
}

/// `threads` threads each lock-and-run their share of `units`, back to
/// back. Timed from the barrier that releases them to the last one's end.
pub fn run_locked(threads: usize, units: u64) -> (Duration, u64) {
    run_contended::<Mutex<Graph>, Graph>(Graph::new(), threads, units)
}

/// [`run_locked`] on any state, under std's mutex.
pub fn run_locked_on<S: State>(g: S, threads: usize, units: u64) -> (Duration, u64) {
    run_contended::<Mutex<S>, S>(g, threads, units)
}

/// [`run_locked`] on any state, under the ticket lock.
pub fn run_ticket_on<S: State>(g: S, threads: usize, units: u64) -> (Duration, u64) {
    run_contended::<Ticket<S>, S>(g, threads, units)
}

/// [`run_bare_on`] on a thread pinned to `cpu`, the pinned runs'
/// baseline.
pub fn run_bare_pinned<S: State>(g: S, cpu: usize, units: u64) -> (Duration, u64) {
    thread::scope(|s| {
        s.spawn(move || {
            pin_to(cpu);
            run_bare_on(g, units)
        })
        .join()
        .expect("no unit panics")
    })
}

/// [`run_locked_on`] with thread `t` pinned to `cpus[t]`, one per CPU.
pub fn run_locked_pinned<S: State>(g: S, cpus: &[usize], units: u64) -> (Duration, u64) {
    run_contended_on::<Mutex<S>, S>(g, cpus.len(), units, cpus)
}

/// [`run_ticket_on`] with thread `t` pinned to `cpus[t]`, one per CPU.
pub fn run_ticket_pinned<S: State>(g: S, cpus: &[usize], units: u64) -> (Duration, u64) {
    run_contended_on::<Ticket<S>, S>(g, cpus.len(), units, cpus)
}

fn run_contended<L: Lock<S>, S: State>(g: S, threads: usize, units: u64) -> (Duration, u64) {
    run_contended_on::<L, S>(g, threads, units, &[])
}

/// Pins thread `t` to `cpus[t]` before it starts, if `cpus` is not empty.
fn pin_nth(cpus: &[usize], t: usize) {
    if let Some(&cpu) = cpus.get(t) {
        pin_to(cpu);
    }
}

/// The contended run, with thread `t` pinned to `cpus[t]` when `cpus` has
/// one for it. Pinning happens before the barrier, outside the timing.
fn run_contended_on<L: Lock<S>, S: State>(
    g: S,
    threads: usize,
    units: u64,
    cpus: &[usize],
) -> (Duration, u64) {
    let m = L::new(g);
    let barrier = Barrier::new(threads + 1);
    let start = thread::scope(|s| {
        for t in 0..threads {
            let (m, barrier) = (&m, &barrier);
            s.spawn(move || {
                pin_nth(cpus, t);
                let range = share(t, threads, units);
                barrier.wait();
                for k in range {
                    m.with(|g| g.propagate(k));
                }
            });
        }
        barrier.wait();
        Instant::now()
    });
    // The scope has joined every thread by here.
    let elapsed = start.elapsed();
    (elapsed, m.into_inner().sink())
}

/// `producers` threads each send their share of `units` through the inbox
/// as fast as they can, and this thread drives: pumps while there is
/// anything queued, parks when there isn't. Timed from the barrier to the
/// last unit run.
pub fn run_queued(producers: usize, units: u64) -> (Duration, u64) {
    run_queued_on(Graph::new(), producers, units)
}

/// [`run_queued`] on any state.
pub fn run_queued_on<S: State>(mut g: S, producers: usize, units: u64) -> (Duration, u64) {
    let inbox = Inbox::new(thread_waker());
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

/// Which lock [`handoffs`] takes.
#[derive(Clone, Copy, Debug)]
pub enum LockKind {
    Mutex,
    Ticket,
    /// [`FutexMutex`], std's mutex copied with its syscalls counted.
    Futex,
}

impl LockKind {
    pub fn name(self) -> &'static str {
        match self {
            LockKind::Mutex => "mutex",
            LockKind::Ticket => "ticket",
            LockKind::Futex => "futex",
        }
    }
}

/// How often a contended lock changed hands, which is how often the state
/// could have migrated: `changes` of `units` went to a different thread
/// from the unit before. And what the workers asked of the kernel while
/// they ran: their voluntary context switches (sleeps), and for
/// [`LockKind::Futex`] its futex calls (zero for the other two).
#[derive(Clone, Copy, Debug)]
pub struct Handoffs {
    pub units: u64,
    pub changes: u64,
    pub longest_streak: u64,
    pub sleeps: u64,
    pub futex: FutexCalls,
}

/// `threads` threads each run `per_thread` units on a fresh `bytes`
/// footprint under `kind`, counting hand-offs. Untimed.
pub fn handoffs(kind: LockKind, bytes: usize, threads: usize, per_thread: u64) -> Handoffs {
    handoffs_on(kind, bytes, threads, &[], per_thread)
}

/// [`handoffs`] with thread `t` pinned to `cpus[t]`, if `cpus` is not empty.
pub fn handoffs_on(
    kind: LockKind,
    bytes: usize,
    threads: usize,
    cpus: &[usize],
    per_thread: u64,
) -> Handoffs {
    match kind {
        LockKind::Mutex => handoffs_under::<Mutex<Tracked>>(bytes, threads, cpus, per_thread),
        LockKind::Ticket => handoffs_under::<Ticket<Tracked>>(bytes, threads, cpus, per_thread),
        LockKind::Futex => handoffs_under::<FutexMutex<Tracked>>(bytes, threads, cpus, per_thread),
    }
}

/// The footprint and, riding in the lock with it, the last unit's thread,
/// the current streak, the longest, and the count of changes.
type Tracked = (Footprint, usize, u64, u64, u64);

fn handoffs_under<L: Lock<Tracked>>(
    bytes: usize,
    threads: usize,
    cpus: &[usize],
    per_thread: u64,
) -> Handoffs {
    let m = L::new((Footprint::new(bytes), usize::MAX, 0, 0, 0));
    let barrier = Barrier::new(threads);
    let (sleeps, futex) = thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (m, barrier) = (&m, &barrier);
                s.spawn(move || {
                    pin_nth(cpus, t);
                    barrier.wait();
                    // Counted from here: the barrier's own sleep is before.
                    let (sleeps, calls) = (voluntary_switches(), futex_calls());
                    for k in t as u64 * per_thread..(t as u64 + 1) * per_thread {
                        m.with(|(g, owner, streak, longest, changes)| {
                            g.propagate(k);
                            if *owner == t {
                                *streak += 1;
                            } else {
                                *streak = 1;
                                *changes += 1;
                            }
                            *owner = t;
                            *longest = (*longest).max(*streak);
                        });
                    }
                    let after = futex_calls();
                    let calls = FutexCalls {
                        waits: after.waits - calls.waits,
                        wakes: after.wakes - calls.wakes,
                        woken: after.woken - calls.woken,
                        wake_ns: after.wake_ns - calls.wake_ns,
                    };
                    (voluntary_switches() - sleeps, calls)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("no unit panics"))
            .fold((0, FutexCalls::default()), |(s, c), (s1, c1)| {
                (s + s1, c + c1)
            })
    });
    let (g, _, _, longest, changes) = m.into_inner();
    assert!(g.consistent());
    Handoffs {
        units: g.units(),
        // The first unit's "change" is from nobody.
        changes: changes - 1,
        longest_streak: longest,
        sleeps,
        futex,
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

    /// The footprint's chain is the graph's propagation, and every
    /// footprint and lock runs every unit once, touching every line.
    #[test]
    fn footprints_agree() {
        let want = expected(1_000);
        for (bytes, label) in FOOTPRINTS {
            let mut f = Footprint::new(bytes);
            assert_eq!(bare(&mut f, 0, 1_000), want, "{label} bare");
            assert!(f.consistent(), "{label} bare");
            let s = Single::of(|| Footprint::new(bytes));
            let mut f = Footprint::new(bytes);
            assert_eq!(queued(&s.inbox, &mut f, 0, 1_000, BURST), want);
            assert!(f.consistent(), "{label} queue");
            let m = Mutex::new(Footprint::new(bytes));
            assert_eq!(locked(&m, 0, 1_000), want, "{label} lock");
            let m: Ticket<Footprint> = Lock::new(Footprint::new(bytes));
            assert_eq!(ticketed(&m, 0, 1_000), want, "{label} ticket");
            assert_eq!(run_bare_on(Footprint::new(bytes), 1_000).1, want);
            for threads in [1, 2, 4, 8] {
                let f = || Footprint::new(bytes);
                assert_eq!(run_locked_on(f(), threads, 1_000).1, want);
                assert_eq!(run_ticket_on(f(), threads, 1_000).1, want);
                assert_eq!(run_queued_on(f(), threads, 1_000).1, want);
            }
            // The ticket lock is fair: with every thread queued, nearly
            // every unit goes to another thread.
            let h = handoffs(LockKind::Ticket, bytes, 4, 250);
            assert_eq!(h.units, 1_000);
            assert!(h.changes > 500, "{label}: {h:?}");
            let h = handoffs(LockKind::Mutex, bytes, 4, 250);
            assert_eq!(h.units, 1_000);
            assert_eq!(h.futex.waits + h.futex.wakes, 0, "std's isn't counted");
            let h = handoffs(LockKind::Futex, bytes, 4, 250);
            assert_eq!(h.units, 1_000);
            // A wake wakes at most one thread.
            assert!(h.futex.woken <= h.futex.wakes, "{label}: {h:?}");
        }
    }

    /// The pinned runs run every unit once, wherever they are placed.
    #[test]
    fn pinned_agree() {
        let want = expected(1_000);
        for (name, cpus) in PLACEMENTS {
            for (bytes, label) in [FOOTPRINTS[0], FOOTPRINTS[2]] {
                let f = || Footprint::new(bytes);
                assert_eq!(run_bare_pinned(f(), cpus[0], 1_000).1, want);
                for threads in [2, 4] {
                    let cpus = &cpus[..threads];
                    let got = run_locked_pinned(f(), cpus, 1_000).1;
                    assert_eq!(got, want, "{name} {label} lock x{threads}");
                    let got = run_ticket_pinned(f(), cpus, 1_000).1;
                    assert_eq!(got, want, "{name} {label} ticket x{threads}");
                    let h = handoffs_on(LockKind::Ticket, bytes, threads, cpus, 250);
                    assert_eq!(h.units, threads as u64 * 250);
                }
            }
        }
    }

    /// The copy of std's mutex excludes: every unit runs once, and under
    /// contention it sleeps and wakes, as the counts claim.
    #[test]
    fn futex_mutex_counts() {
        let want = expected(4_000);
        let got = run_contended::<FutexMutex<Graph>, Graph>(Graph::new(), 4, 4_000).1;
        assert_eq!(got, want);
        let h = handoffs(LockKind::Futex, 0, 4, 2_000);
        assert!(h.futex.wakes > 0, "{h:?}");
        assert!(h.futex.woken <= h.futex.wakes, "{h:?}");
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
