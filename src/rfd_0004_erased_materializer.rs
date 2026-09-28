//! RFD 4, fusion: does erasing a fused chain at its materializer remove
//! F36's per-chain compile blow-up, and what does it cost per event?
//!
//! F36: every materializer is compiled again for each nested chain type, so
//! a program that builds chains from data pays for every chain shape it can
//! make. This module is a small engine in the spike's shape (`Source` with
//! an associated `Event`, adapters that nest their types, materializers that
//! make one node with an ops table of fn pointers), and four ways to hand a
//! chain to a materializer:
//!
//! - `baseline`: the materializer is generic over the chain type, as in the
//!   spike, so it is compiled once per chain type.
//! - `boxed`: the chain is erased to a [`Fused`], one
//!   `Box<dyn FnMut(&mut Cx) -> Option<A>>`, and the materializer is
//!   compiled once per event type.
//! - `fnptr`: the chain is erased to a [`Machine`], Lustre's machine with
//!   `self` (Biernacki et al., pp. 5, 8): the chain is a state struct and a
//!   monomorphized step function reached through a fn pointer, and the
//!   materializer is compiled once per event type.
//! - `flat`: each adapter normalizes the chain CCNF-style into one flat
//!   [`Flat`] node, a state tuple and one step closure, and the generic
//!   materializer takes that. The nesting moves from the adapter types into
//!   the state tuple and the composed closure.
//!
//! The erased signature takes the build context, not the event: the spike's
//! chains pull their own head (a take from a linear stream, a clone from a
//! shared one), and `snapshot` and `gate` read cells through it. It is still
//! one indirect call per chain per event, which is the price 09-values names.
//!
//! Every design's node runs through the same generic node code; the only
//! difference is the chain carrier each materializer is instantiated with.
//! The compile-time binary includes this file verbatim in the crates it
//! generates, so the benches and the build times measure the same code.

use std::any::Any;
use std::marker::PhantomData;
use std::ptr::NonNull;

// ----- the engine -----

/// What a node does, one table per node type, as in the spike's `Ops`.
pub struct Ops {
    eval: fn(&mut dyn Any, &mut Cx<'_>, usize),
    commit: fn(&mut dyn Any),
}

fn no_eval(_: &mut dyn Any, _: &mut Cx<'_>, _: usize) {}

fn no_commit(_: &mut dyn Any) {}

/// A stream's commit: the event nobody took is dropped.
fn clear<A: 'static>(data: &mut dyn Any) {
    *slot::<A>(data) = None;
}

/// A cell's commit: the pending value, if the cell stepped, replaces it.
fn commit_cell<A: 'static>(data: &mut dyn Any) {
    let cell = cell_mut::<A>(data);
    if let Some(v) = cell.pending.take() {
        cell.value = v;
    }
}

struct CellData<A> {
    value: A,
    pending: Option<A>,
}

fn slot<A: 'static>(data: &mut dyn Any) -> &mut Option<A> {
    data.downcast_mut()
        .expect("probe engine: a stream slot has its event type")
}

fn cell_ref<A: 'static>(data: &dyn Any) -> &CellData<A> {
    data.downcast_ref()
        .expect("probe engine: a cell has its value type")
}

fn cell_mut<A: 'static>(data: &mut dyn Any) -> &mut CellData<A> {
    data.downcast_mut()
        .expect("probe engine: a cell has its value type")
}

fn part<P: 'static>(parts: &mut dyn Any) -> &mut P {
    parts
        .downcast_mut()
        .expect("probe engine: a node's parts have its type")
}

/// The graph and its build context in one: nodes run in creation order,
/// which is a topological order since a chain reads only older nodes.
#[derive(Default)]
pub struct Graph {
    data: Vec<Box<dyn Any>>,
    parts: Vec<Box<dyn Any>>,
    ops: Vec<&'static Ops>,
    /// What each node's chain reads besides its dependency (RFD 3), found by
    /// tracing the chain once when it is materialized.
    reach: Vec<Vec<u32>>,
}

/// What a chain reads while it runs: every node's data.
pub struct Cx<'a> {
    data: &'a mut [Box<dyn Any>],
}

impl Cx<'_> {
    fn take<A: 'static>(&mut self, index: u32) -> Option<A> {
        slot::<A>(&mut *self.data[index as usize]).take()
    }

    fn cloned<A: Clone + 'static>(&self, index: u32) -> Option<A> {
        self.data[index as usize]
            .downcast_ref::<Option<A>>()
            .expect("probe engine: a shared slot has its event type")
            .clone()
    }

    /// A cell's value from before the instant.
    pub fn sample<A: 'static>(&self, index: u32) -> &A {
        &cell_ref::<A>(&*self.data[index as usize]).value
    }

    fn put_pending<A: 'static>(&mut self, me: usize, v: A) {
        cell_mut::<A>(&mut *self.data[me]).pending = Some(v);
    }

    fn put_event<A: 'static>(&mut self, me: usize, v: A) {
        *slot::<A>(&mut *self.data[me]) = Some(v);
    }
}

const CONSTANT: Ops = Ops {
    eval: no_eval,
    commit: no_commit,
};

struct InputNode<A>(PhantomData<A>);
impl<A: 'static> InputNode<A> {
    const OPS: Ops = Ops {
        eval: no_eval,
        commit: clear::<A>,
    };
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    fn materialize(
        &mut self,
        data: Box<dyn Any>,
        parts: Box<dyn Any>,
        ops: &'static Ops,
        dependency: u32,
        reach: Vec<u32>,
    ) -> u32 {
        let n = self.data.len();
        assert!(
            (dependency as usize) < n,
            "probe engine: a chain reads an older node"
        );
        self.data.push(data);
        self.parts.push(parts);
        self.ops.push(ops);
        self.reach.push(reach);
        n as u32
    }

    fn leaf(&mut self, data: Box<dyn Any>, ops: &'static Ops) -> u32 {
        let n = self.data.len();
        self.data.push(data);
        self.parts.push(Box::new(()));
        self.ops.push(ops);
        self.reach.push(Vec::new());
        n as u32
    }

    /// A linear stream of events sent from outside, and its handle.
    pub fn input<A: 'static>(&mut self) -> (Stream<A>, Input<A>) {
        let n = self.leaf(Box::new(None::<A>), &InputNode::<A>::OPS);
        (Stream::at(n), Input::at(n))
    }

    /// A shared stream of events sent from outside, and its handle.
    pub fn shared_input<A: Clone + 'static>(&mut self) -> (Shared<A>, Input<A>) {
        let n = self.leaf(Box::new(None::<A>), &InputNode::<A>::OPS);
        (Shared::at(n), Input::at(n))
    }

    pub fn constant_cell<A: 'static>(&mut self, value: A) -> Cell<A> {
        let data = Box::new(CellData {
            value,
            pending: None,
        });
        Cell::at(self.leaf(data, &CONSTANT))
    }

    pub fn constant_state<A: 'static>(&mut self, value: A) -> State<A> {
        let data = Box::new(CellData {
            value,
            pending: None,
        });
        State::at(self.leaf(data, &CONSTANT))
    }

    pub fn send<A: 'static>(&mut self, input: &Input<A>, v: A) {
        *slot::<A>(&mut *self.data[input.index as usize]) = Some(v);
    }

    /// One transaction: every node evaluates, then every node commits.
    pub fn run(&mut self) {
        for (me, parts) in self.parts.iter_mut().enumerate() {
            let mut cx = Cx {
                data: &mut self.data,
            };
            (self.ops[me].eval)(&mut **parts, &mut cx, me);
        }
        for (data, ops) in self.data.iter_mut().zip(&self.ops) {
            (ops.commit)(&mut **data);
        }
    }

    pub fn sample<A: 'static>(&self, cell: Cell<A>) -> &A {
        &cell_ref::<A>(&*self.data[cell.index as usize]).value
    }

    /// How many nodes' chains read a cell besides their dependency.
    pub fn reaching(&self) -> usize {
        self.reach.iter().filter(|r| !r.is_empty()).count()
    }
}

// ----- tokens -----

/// A linear stream: one consumer takes its event.
pub struct Stream<A> {
    index: u32,
    _event: PhantomData<fn() -> A>,
}

macro_rules! copy_token {
    ($($name:ident),*) => {$(
        pub struct $name<A> {
            index: u32,
            _event: PhantomData<fn() -> A>,
        }
        impl<A> Clone for $name<A> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<A> Copy for $name<A> {}
    )*};
}

copy_token!(Shared, Cell, State, Input);

macro_rules! at {
    ($($name:ident),*) => {$(
        impl<A> $name<A> {
            fn at(index: u32) -> Self {
                $name { index, _event: PhantomData }
            }
        }
    )*};
}

at!(Stream, Shared, Cell, State, Input);

/// A cell a chain reads without depending on it: a `Cell` or a `State`,
/// typed apart as in the spike (F37).
pub trait CellRef: Copy + 'static {
    type Value: 'static;
    fn index(&self) -> u32;
}
impl<A: 'static> CellRef for Cell<A> {
    type Value = A;
    fn index(&self) -> u32 {
        self.index
    }
}
impl<A: 'static> CellRef for State<A> {
    type Value = A;
    fn index(&self) -> u32 {
        self.index
    }
}

/// What a value kept by a node reaches, found once at build (RFD 3).
pub trait Trace {
    fn trace(&self, reach: &mut Vec<u32>);
}
macro_rules! leaf_trace {
    ($($t:ty),*) => {$(
        impl Trace for $t {
            fn trace(&self, _: &mut Vec<u32>) {}
        }
    )*};
}
leaf_trace!((), bool, i64, u64);
impl<A: Trace, B: Trace> Trace for (A, B) {
    fn trace(&self, reach: &mut Vec<u32>) {
        self.0.trace(reach);
        self.1.trace(reach);
    }
}
impl<A> Trace for Cell<A> {
    fn trace(&self, reach: &mut Vec<u32>) {
        reach.push(self.index);
    }
}
impl<A> Trace for State<A> {
    fn trace(&self, reach: &mut Vec<u32>) {
        reach.push(self.index);
    }
}

/// A closure kept in a flat chain's state: opaque to the trace, as every
/// closure is.
pub struct Opaque<F>(F);
impl<F> Trace for Opaque<F> {
    fn trace(&self, _: &mut Vec<u32>) {}
}

// ----- chains -----

/// Anything that yields events: a node or a chain of adapters, the spike's
/// `Source` without its modes.
pub trait Source: Sized + 'static {
    type Event: 'static;

    /// The one node the chain reads events from.
    fn dependency(&self) -> u32;

    /// The cells the chain reads.
    fn trace(&self, reach: &mut Vec<u32>);

    /// Runs the fused chain for this instant.
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<Self::Event>;

    fn map<B, F>(self, f: F) -> Map<Self, F>
    where
        F: Fn(Self::Event) -> B + 'static,
    {
        Map { source: self, f }
    }

    fn filter<P>(self, predicate: P) -> Filter<Self, P>
    where
        P: Fn(&Self::Event) -> bool + 'static,
    {
        Filter {
            source: self,
            predicate,
        }
    }

    fn filter_map<B, F>(self, f: F) -> FilterMap<Self, F>
    where
        F: Fn(Self::Event) -> Option<B> + 'static,
    {
        FilterMap { source: self, f }
    }

    fn map_to<B>(self, value: B) -> MapTo<Self, B>
    where
        B: Clone + Trace + 'static,
    {
        MapTo {
            source: self,
            value,
        }
    }

    fn snapshot<C, B, F>(self, cell: C, f: F) -> Snapshot<Self, C, F>
    where
        C: CellRef,
        F: Fn(Self::Event, &C::Value) -> B + 'static,
    {
        Snapshot {
            source: self,
            cell,
            f,
        }
    }

    fn gate<C>(self, cell: C) -> Gate<Self, C>
    where
        C: CellRef<Value = bool>,
    {
        Gate { source: self, cell }
    }

    fn once(self) -> Once<Self> {
        Once {
            source: self,
            done: false,
        }
    }
}

impl<A: 'static> Source for Stream<A> {
    type Event = A;
    fn dependency(&self) -> u32 {
        self.index
    }
    fn trace(&self, _: &mut Vec<u32>) {}
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<A> {
        cx.take(self.index)
    }
}

impl<A: Clone + 'static> Source for Shared<A> {
    type Event = A;
    fn dependency(&self) -> u32 {
        self.index
    }
    fn trace(&self, _: &mut Vec<u32>) {}
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<A> {
        cx.cloned(self.index)
    }
}

pub struct Map<S, F> {
    source: S,
    f: F,
}
impl<S: Source, B: 'static, F: Fn(S::Event) -> B + 'static> Source for Map<S, F> {
    type Event = B;
    fn dependency(&self) -> u32 {
        self.source.dependency()
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        self.source.trace(reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<B> {
        self.source.pull(cx).map(&self.f)
    }
}

pub struct Filter<S, P> {
    source: S,
    predicate: P,
}
impl<S: Source, P: Fn(&S::Event) -> bool + 'static> Source for Filter<S, P> {
    type Event = S::Event;
    fn dependency(&self) -> u32 {
        self.source.dependency()
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        self.source.trace(reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<S::Event> {
        self.source.pull(cx).filter(|e| (self.predicate)(e))
    }
}

pub struct FilterMap<S, F> {
    source: S,
    f: F,
}
impl<S: Source, B: 'static, F: Fn(S::Event) -> Option<B> + 'static> Source for FilterMap<S, F> {
    type Event = B;
    fn dependency(&self) -> u32 {
        self.source.dependency()
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        self.source.trace(reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<B> {
        self.source.pull(cx).and_then(&self.f)
    }
}

pub struct MapTo<S, B> {
    source: S,
    value: B,
}
impl<S: Source, B: Clone + Trace + 'static> Source for MapTo<S, B> {
    type Event = B;
    fn dependency(&self) -> u32 {
        self.source.dependency()
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        self.value.trace(reach);
        self.source.trace(reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<B> {
        self.source.pull(cx).map(|_| self.value.clone())
    }
}

pub struct Snapshot<S, C, F> {
    source: S,
    cell: C,
    f: F,
}
impl<S, C, B, F> Source for Snapshot<S, C, F>
where
    S: Source,
    C: CellRef,
    B: 'static,
    F: Fn(S::Event, &C::Value) -> B + 'static,
{
    type Event = B;
    fn dependency(&self) -> u32 {
        self.source.dependency()
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        reach.push(self.cell.index());
        self.source.trace(reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<B> {
        let a = self.source.pull(cx)?;
        // The value before the instant: a read, not a dependency.
        Some((self.f)(a, cx.sample(self.cell.index())))
    }
}

pub struct Gate<S, C> {
    source: S,
    cell: C,
}
impl<S: Source, C: CellRef<Value = bool>> Source for Gate<S, C> {
    type Event = S::Event;
    fn dependency(&self) -> u32 {
        self.source.dependency()
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        reach.push(self.cell.index());
        self.source.trace(reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<S::Event> {
        let a = self.source.pull(cx)?;
        cx.sample::<bool>(self.cell.index()).then_some(a)
    }
}

pub struct Once<S> {
    source: S,
    done: bool,
}
impl<S: Source> Source for Once<S> {
    type Event = S::Event;
    fn dependency(&self) -> u32 {
        self.source.dependency()
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        self.source.trace(reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<S::Event> {
        if self.done {
            return None;
        }
        let a = self.source.pull(cx)?;
        self.done = true;
        Some(a)
    }
}

fn chain_reach<S: Source>(chain: &S) -> (u32, Vec<u32>) {
    let mut reach = Vec::new();
    chain.trace(&mut reach);
    (chain.dependency(), reach)
}

// ----- materializers, generic over the chain: the baseline -----
//
// Each is compiled once per chain type it is called with. The erased
// designs call these same functions with their carrier as the chain.

struct HoldNode<S>(PhantomData<S>);
impl<S: Source> HoldNode<S> {
    const OPS: Ops = Ops {
        eval: eval_hold::<S>,
        commit: commit_cell::<S::Event>,
    };
}
fn eval_hold<S: Source>(parts: &mut dyn Any, cx: &mut Cx<'_>, me: usize) {
    if let Some(v) = part::<S>(parts).pull(cx) {
        cx.put_pending(me, v);
    }
}

/// A cell holding the latest event.
pub fn hold<S: Source>(b: &mut Graph, chain: S, initial: S::Event) -> Cell<S::Event> {
    let (dependency, reach) = chain_reach(&chain);
    let data = Box::new(CellData {
        value: initial,
        pending: None,
    });
    let ops = &HoldNode::<S>::OPS;
    Cell::at(b.materialize(data, Box::new(chain), ops, dependency, reach))
}

struct AccumulateNode<S, St, F>(PhantomData<(S, St, F)>);
impl<S: Source, St: 'static, F: Fn(S::Event, &St) -> St + 'static> AccumulateNode<S, St, F> {
    const OPS: Ops = Ops {
        eval: eval_accumulate::<S, St, F>,
        commit: commit_cell::<St>,
    };
}
fn eval_accumulate<S, St, F>(parts: &mut dyn Any, cx: &mut Cx<'_>, me: usize)
where
    S: Source,
    St: 'static,
    F: Fn(S::Event, &St) -> St + 'static,
{
    let (chain, f) = part::<(S, F)>(parts);
    let Some(event) = chain.pull(cx) else {
        return;
    };
    let next = f(event, cx.sample::<St>(me as u32));
    cx.put_pending(me, next);
}

/// Sodium's `accum`: `f` reads the state from before the instant.
pub fn accumulate<S, St, F>(b: &mut Graph, chain: S, initial: St, f: F) -> Cell<St>
where
    S: Source,
    St: 'static,
    F: Fn(S::Event, &St) -> St + 'static,
{
    let (dependency, reach) = chain_reach(&chain);
    let data = Box::new(CellData {
        value: initial,
        pending: None,
    });
    let ops = &AccumulateNode::<S, St, F>::OPS;
    Cell::at(b.materialize(data, Box::new((chain, f)), ops, dependency, reach))
}

struct ScanNode<S, St, B, F>(PhantomData<(S, St, B, F)>);
impl<S, St, B, F> ScanNode<S, St, B, F>
where
    S: Source,
    St: 'static,
    B: 'static,
    F: Fn(S::Event, &St) -> (B, St) + 'static,
{
    const OPS: Ops = Ops {
        eval: eval_scan::<S, St, B, F>,
        commit: clear::<B>,
    };
}
fn eval_scan<S, St, B, F>(parts: &mut dyn Any, cx: &mut Cx<'_>, me: usize)
where
    S: Source,
    St: 'static,
    B: 'static,
    F: Fn(S::Event, &St) -> (B, St) + 'static,
{
    let (chain, f, state) = part::<(S, F, St)>(parts);
    let Some(event) = chain.pull(cx) else {
        return;
    };
    let (out, next) = f(event, state);
    *state = next;
    cx.put_event(me, out);
}

/// `Iterator::scan`: a running state and an output per event.
pub fn scan<S, St, B, F>(b: &mut Graph, chain: S, initial: St, f: F) -> Stream<B>
where
    S: Source,
    St: 'static,
    B: 'static,
    F: Fn(S::Event, &St) -> (B, St) + 'static,
{
    let (dependency, reach) = chain_reach(&chain);
    let ops = &ScanNode::<S, St, B, F>::OPS;
    let parts = Box::new((chain, f, initial));
    Stream::at(b.materialize(Box::new(None::<B>), parts, ops, dependency, reach))
}

struct ChainNode<S>(PhantomData<S>);
impl<S: Source> ChainNode<S> {
    const OPS: Ops = Ops {
        eval: eval_chain::<S>,
        commit: clear::<S::Event>,
    };
}
fn eval_chain<S: Source>(parts: &mut dyn Any, cx: &mut Cx<'_>, me: usize) {
    if let Some(v) = part::<S>(parts).pull(cx) {
        cx.put_event(me, v);
    }
}

fn chain_node<S: Source>(b: &mut Graph, chain: S) -> u32 {
    let (dependency, reach) = chain_reach(&chain);
    let ops = &ChainNode::<S>::OPS;
    let data = Box::new(None::<S::Event>);
    b.materialize(data, Box::new(chain), ops, dependency, reach)
}

/// A chain as a linear stream with an identity of its own.
pub fn node<S: Source>(b: &mut Graph, chain: S) -> Stream<S::Event> {
    Stream::at(chain_node(b, chain))
}

/// Explicit fan-out: each consumer clones the event.
pub fn share<S: Source>(b: &mut Graph, chain: S) -> Shared<S::Event>
where
    S::Event: Clone,
{
    Shared::at(chain_node(b, chain))
}

struct ListenNode<S, F>(PhantomData<(S, F)>);
impl<S: Source, F: FnMut(&S::Event) + 'static> ListenNode<S, F> {
    const OPS: Ops = Ops {
        eval: eval_listen::<S, F>,
        commit: no_commit,
    };
}
fn eval_listen<S: Source, F: FnMut(&S::Event) + 'static>(
    parts: &mut dyn Any,
    cx: &mut Cx<'_>,
    _: usize,
) {
    let (chain, f) = part::<(S, F)>(parts);
    if let Some(v) = chain.pull(cx) {
        f(&v);
    }
}

/// Calls `f` with every event.
pub fn listen<S: Source, F: FnMut(&S::Event) + 'static>(b: &mut Graph, chain: S, f: F) {
    let (dependency, reach) = chain_reach(&chain);
    let ops = &ListenNode::<S, F>::OPS;
    b.materialize(Box::new(()), Box::new((chain, f)), ops, dependency, reach);
}

// ----- `boxed`: the chain as one boxed closure -----

/// A fused chain erased to one boxed closure. The chain inside is still one
/// monomorphized closure; the materializer sees only the event type.
pub struct Fused<A> {
    dependency: u32,
    reach: Vec<u32>,
    run: Run<A>,
}

/// The one indirect call per chain per event.
pub type Run<A> = Box<dyn FnMut(&mut Cx<'_>) -> Option<A>>;

impl<A: 'static> Fused<A> {
    pub fn new<S: Source<Event = A>>(mut chain: S) -> Self {
        let (dependency, reach) = chain_reach(&chain);
        Fused {
            dependency,
            reach,
            run: Box::new(move |cx: &mut Cx<'_>| chain.pull(cx)),
        }
    }
}

impl<A: 'static> Source for Fused<A> {
    type Event = A;
    fn dependency(&self) -> u32 {
        self.dependency
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        reach.extend_from_slice(&self.reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<A> {
        (self.run)(cx)
    }
}

// ----- `fnptr`: the chain as a machine with `self` -----

/// A fused chain as Lustre's machine: its state, the chain itself, behind a
/// pointer, and its step, compiled once per chain type, behind a fn
/// pointer. No vtable: a node holds the two pointers and a drop.
pub struct Machine<A> {
    dependency: u32,
    reach: Vec<u32>,
    state: NonNull<()>,
    step: unsafe fn(NonNull<()>, &mut Cx<'_>) -> Option<A>,
    drop: unsafe fn(NonNull<()>),
}

/// # Safety
///
/// `state` is the `Box<S>` a `Machine::new::<S>` leaked, not yet dropped,
/// and nothing else borrows it.
unsafe fn step<S: Source>(state: NonNull<()>, cx: &mut Cx<'_>) -> Option<S::Event> {
    // SAFETY: the caller's contract.
    unsafe { state.cast::<S>().as_mut() }.pull(cx)
}

/// # Safety
///
/// As [`step`], and the state is not used again.
unsafe fn drop_state<S>(state: NonNull<()>) {
    // SAFETY: the caller's contract.
    drop(unsafe { Box::from_raw(state.cast::<S>().as_ptr()) });
}

impl<A: 'static> Machine<A> {
    pub fn new<S: Source<Event = A>>(chain: S) -> Self {
        let (dependency, reach) = chain_reach(&chain);
        Machine {
            dependency,
            reach,
            state: NonNull::from(Box::leak(Box::new(chain))).cast(),
            step: step::<S>,
            drop: drop_state::<S>,
        }
    }
}

impl<A> Drop for Machine<A> {
    fn drop(&mut self) {
        // SAFETY: `state`, `step` and `drop` were made together in `new`
        // for one `S`, and this is the machine's only drop.
        unsafe { (self.drop)(self.state) }
    }
}

impl<A: 'static> Source for Machine<A> {
    type Event = A;
    fn dependency(&self) -> u32 {
        self.dependency
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        reach.extend_from_slice(&self.reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<A> {
        // SAFETY: made together in `new`, not dropped while `self` lives,
        // and `&mut self` makes the borrow unique.
        unsafe { (self.step)(self.state, cx) }
    }
}

// ----- the erased materializers -----
//
// The public materializer is a thin generic shim that erases the chain,
// compiled per chain type; the materializer proper is compiled once per
// event type. It is `inline(never)`, since the erasure is only worth
// anything if the compiler doesn't copy it back into every shim.

macro_rules! erased {
    ($design:ident, $carrier:ident) => {
        pub mod $design {
            use super::{Cell, Graph, Shared, Source, Stream, $carrier};

            pub fn hold<S: Source>(b: &mut Graph, chain: S, initial: S::Event) -> Cell<S::Event> {
                erased::hold(b, $carrier::new(chain), initial)
            }

            pub fn accumulate<S, St, F>(b: &mut Graph, chain: S, initial: St, f: F) -> Cell<St>
            where
                S: Source,
                St: 'static,
                F: Fn(S::Event, &St) -> St + 'static,
            {
                erased::accumulate(b, $carrier::new(chain), initial, f)
            }

            pub fn scan<S, St, B, F>(b: &mut Graph, chain: S, initial: St, f: F) -> Stream<B>
            where
                S: Source,
                St: 'static,
                B: 'static,
                F: Fn(S::Event, &St) -> (B, St) + 'static,
            {
                erased::scan(b, $carrier::new(chain), initial, f)
            }

            pub fn node<S: Source>(b: &mut Graph, chain: S) -> Stream<S::Event> {
                erased::node(b, $carrier::new(chain))
            }

            pub fn share<S: Source>(b: &mut Graph, chain: S) -> Shared<S::Event>
            where
                S::Event: Clone,
            {
                erased::share(b, $carrier::new(chain))
            }

            pub fn listen<S, F>(b: &mut Graph, chain: S, f: F)
            where
                S: Source,
                F: FnMut(&S::Event) + 'static,
            {
                erased::listen(b, $carrier::new(chain), f)
            }

            mod erased {
                use super::super as engine;
                use super::{Cell, Graph, Shared, Stream, $carrier};

                #[inline(never)]
                pub fn hold<A: 'static>(b: &mut Graph, chain: $carrier<A>, initial: A) -> Cell<A> {
                    engine::hold(b, chain, initial)
                }

                #[inline(never)]
                pub fn accumulate<A, St, F>(
                    b: &mut Graph,
                    chain: $carrier<A>,
                    initial: St,
                    f: F,
                ) -> Cell<St>
                where
                    A: 'static,
                    St: 'static,
                    F: Fn(A, &St) -> St + 'static,
                {
                    engine::accumulate(b, chain, initial, f)
                }

                #[inline(never)]
                pub fn scan<A, St, B, F>(
                    b: &mut Graph,
                    chain: $carrier<A>,
                    initial: St,
                    f: F,
                ) -> Stream<B>
                where
                    A: 'static,
                    St: 'static,
                    B: 'static,
                    F: Fn(A, &St) -> (B, St) + 'static,
                {
                    engine::scan(b, chain, initial, f)
                }

                #[inline(never)]
                pub fn node<A: 'static>(b: &mut Graph, chain: $carrier<A>) -> Stream<A> {
                    engine::node(b, chain)
                }

                #[inline(never)]
                pub fn share<A: Clone + 'static>(b: &mut Graph, chain: $carrier<A>) -> Shared<A> {
                    engine::share(b, chain)
                }

                #[inline(never)]
                pub fn listen<A, F>(b: &mut Graph, chain: $carrier<A>, f: F)
                where
                    A: 'static,
                    F: FnMut(&A) + 'static,
                {
                    engine::listen(b, chain, f)
                }
            }
        }
    };
}

erased!(boxed, Fused);
erased!(fnptr, Machine);

// ----- `flat`: the chain normalized into one (state, step) node -----

/// A chain in CCNF's canonical shape (Liu et al., pp. 5-6): the head it
/// pulls, one state tuple holding every adapter's state and function, and
/// one step closure over it. Each adapter composes the step at once, so
/// there is no adapter type to nest; the nesting is in `St` and `F`.
pub struct Flat<H, St, F, B> {
    head: H,
    state: St,
    step: F,
    _event: PhantomData<fn() -> B>,
}

/// The step of the chain of no adapters.
pub type Pass<A> = fn(A, &mut (), &Cx<'_>) -> Option<A>;

fn pass<A>(a: A, _: &mut (), _: &Cx<'_>) -> Option<A> {
    Some(a)
}

impl<H: Source> Flat<H, (), Pass<H::Event>, H::Event> {
    /// The chain of no adapters over a node.
    pub fn new(head: H) -> Self {
        Flat {
            head,
            state: (),
            step: pass::<H::Event>,
            _event: PhantomData,
        }
    }
}

impl<H, St, F, B> Source for Flat<H, St, F, B>
where
    H: Source,
    St: Trace + 'static,
    B: 'static,
    F: Fn(H::Event, &mut St, &Cx<'_>) -> Option<B> + 'static,
{
    type Event = B;
    fn dependency(&self) -> u32 {
        self.head.dependency()
    }
    fn trace(&self, reach: &mut Vec<u32>) {
        self.state.trace(reach);
        self.head.trace(reach)
    }
    fn pull(&mut self, cx: &mut Cx<'_>) -> Option<B> {
        let a = self.head.pull(cx)?;
        (self.step)(a, &mut self.state, cx)
    }
}

/// Appends one adapter: its state goes on the tuple, its step after the
/// chain's. Neither step captures anything but the other.
#[allow(clippy::type_complexity)]
fn then<H, St, F, B, St2, G, C>(
    flat: Flat<H, St, F, B>,
    state: St2,
    g: G,
) -> Flat<H, (St, St2), impl Fn(H::Event, &mut (St, St2), &Cx<'_>) -> Option<C> + 'static, C>
where
    H: Source,
    St: 'static,
    St2: 'static,
    F: Fn(H::Event, &mut St, &Cx<'_>) -> Option<B> + 'static,
    G: Fn(B, &mut St2, &Cx<'_>) -> Option<C> + 'static,
{
    let f = flat.step;
    Flat {
        head: flat.head,
        state: (flat.state, state),
        step: move |a: H::Event, s: &mut (St, St2), cx: &Cx<'_>| {
            let b = f(a, &mut s.0, cx)?;
            g(b, &mut s.1, cx)
        },
        _event: PhantomData,
    }
}

/// The adapters of a flat chain. Named like `Source`'s, so call them by
/// path: `Flatten::map(chain, f)`.
pub trait Flatten: Source {
    fn map<C: 'static, G: Fn(Self::Event) -> C + 'static>(self, g: G) -> impl Flatten<Event = C>;
    fn filter<P: Fn(&Self::Event) -> bool + 'static>(
        self,
        predicate: P,
    ) -> impl Flatten<Event = Self::Event>;
    fn filter_map<C: 'static, G: Fn(Self::Event) -> Option<C> + 'static>(
        self,
        g: G,
    ) -> impl Flatten<Event = C>;
    fn map_to<C: Clone + Trace + 'static>(self, value: C) -> impl Flatten<Event = C>;
    fn snapshot<Q, C, G>(self, cell: Q, g: G) -> impl Flatten<Event = C>
    where
        Q: CellRef + Trace,
        C: 'static,
        G: Fn(Self::Event, &Q::Value) -> C + 'static;
    fn gate<Q: CellRef<Value = bool> + Trace>(self, cell: Q) -> impl Flatten<Event = Self::Event>;
    fn once(self) -> impl Flatten<Event = Self::Event>;
}

impl<H, St, F, B> Flatten for Flat<H, St, F, B>
where
    H: Source,
    St: Trace + 'static,
    B: 'static,
    F: Fn(H::Event, &mut St, &Cx<'_>) -> Option<B> + 'static,
{
    fn map<C: 'static, G: Fn(B) -> C + 'static>(self, g: G) -> impl Flatten<Event = C> {
        then(self, Opaque(g), |b, s: &mut Opaque<G>, _: &Cx<'_>| {
            Some((s.0)(b))
        })
    }

    fn filter<P: Fn(&B) -> bool + 'static>(self, predicate: P) -> impl Flatten<Event = B> {
        then(
            self,
            Opaque(predicate),
            |b, s: &mut Opaque<P>, _: &Cx<'_>| {
                if (s.0)(&b) { Some(b) } else { None }
            },
        )
    }

    fn filter_map<C: 'static, G: Fn(B) -> Option<C> + 'static>(
        self,
        g: G,
    ) -> impl Flatten<Event = C> {
        then(self, Opaque(g), |b, s: &mut Opaque<G>, _: &Cx<'_>| (s.0)(b))
    }

    fn map_to<C: Clone + Trace + 'static>(self, value: C) -> impl Flatten<Event = C> {
        then(self, value, |_, s: &mut C, _: &Cx<'_>| Some(s.clone()))
    }

    fn snapshot<Q, C, G>(self, cell: Q, g: G) -> impl Flatten<Event = C>
    where
        Q: CellRef + Trace,
        C: 'static,
        G: Fn(B, &Q::Value) -> C + 'static,
    {
        then(
            self,
            (cell, Opaque(g)),
            |b, s: &mut (Q, Opaque<G>), cx: &Cx<'_>| Some((s.1.0)(b, cx.sample(s.0.index()))),
        )
    }

    fn gate<Q: CellRef<Value = bool> + Trace>(self, cell: Q) -> impl Flatten<Event = B> {
        then(self, cell, |b, s: &mut Q, cx: &Cx<'_>| {
            if *cx.sample::<bool>(s.index()) {
                Some(b)
            } else {
                None
            }
        })
    }

    fn once(self) -> impl Flatten<Event = B> {
        then(self, false, |b, done: &mut bool, _: &Cx<'_>| {
            if *done {
                None
            } else {
                *done = true;
                Some(b)
            }
        })
    }
}

// ----- the run-time workload -----

/// The four designs, in the order the benches and the binary print them.
#[derive(Clone, Copy, Debug)]
pub enum Design {
    Baseline,
    Boxed,
    Fnptr,
    Flat,
}

impl Design {
    pub const ALL: [Design; 4] = [Design::Baseline, Design::Boxed, Design::Fnptr, Design::Flat];

    pub fn name(self) -> &'static str {
        match self {
            Design::Baseline => "baseline",
            Design::Boxed => "boxed",
            Design::Fnptr => "fnptr",
            Design::Flat => "flat",
        }
    }
}

fn scramble(x: u64) -> u64 {
    x.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17)
}

fn keep(x: &u64) -> bool {
    // Keeps three events in four.
    x & 3 != 0
}

fn mix(x: u64, c: &u64) -> u64 {
    x ^ c
}

fn three(s: Stream<u64>, cell: Cell<u64>) -> impl Source<Event = u64> {
    s.map(scramble).filter(keep).snapshot(cell, mix)
}

/// `input.map(scramble).filter(keep).snapshot(cell, mix).hold(b, 0)`: a
/// chain of three adapters and one materializer, in a graph of three nodes
/// (the input, the snapshotted cell, the hold), built one way per design.
pub struct ThreeAdapters {
    graph: Graph,
    input: Input<u64>,
    out: Cell<u64>,
}

impl ThreeAdapters {
    pub fn new(design: Design) -> Self {
        let mut g = Graph::new();
        let (s, input) = g.input::<u64>();
        let cell = g.constant_cell(0x5555_u64);
        let out = match design {
            Design::Baseline => hold(&mut g, three(s, cell), 0),
            Design::Boxed => boxed::hold(&mut g, three(s, cell), 0),
            Design::Fnptr => fnptr::hold(&mut g, three(s, cell), 0),
            Design::Flat => {
                // The same three adapters over the same head, normalized.
                let flat = Flatten::map(Flat::new(s), scramble);
                let flat = Flatten::filter(flat, keep);
                hold(&mut g, Flatten::snapshot(flat, cell, mix), 0)
            }
        };
        ThreeAdapters {
            graph: g,
            input,
            out,
        }
    }

    /// One transaction carrying one event.
    pub fn event(&mut self, x: u64) {
        self.graph.send(&self.input, x);
        self.graph.run();
    }

    /// `n` transactions, and the value the hold ends with.
    pub fn events(&mut self, n: u64) -> u64 {
        for x in 0..n {
            self.event(std::hint::black_box(x));
        }
        self.value()
    }

    pub fn value(&self) -> u64 {
        *self.graph.sample(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_design_computes_the_same_value() {
        let values: Vec<u64> = Design::ALL
            .iter()
            .map(|&d| ThreeAdapters::new(d).events(1000))
            .collect();
        assert!(values.iter().all(|&v| v == values[0]), "{values:?}");
        assert_ne!(values[0], 0);
    }
}
