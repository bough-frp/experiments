//! The toy runtime of `rfd-0003-brand-erasure`: the `brand` design of
//! `rfd-0003-branded-captures`, with its rebranding done in safe code.
//!
//! Every token carries an invariant lifetime `'g`, fresh for each
//! `Runtime::mutate`, and graph closures are `'static`, so a closure can't
//! capture a token. The earlier skeleton left `open` as `todo!()`: gc-arena
//! rebrands with `unsafe`, and Bough's core forbids it. Here nothing is
//! `unsafe`, and the runtime is real: tokens are (index, generation, graph)
//! into an arena of nodes, values flow through the nodes, a collection frees
//! what no guard reaches, and a freed slot bumps its generation.
//!
//! How the brand is changed safely:
//!
//! - `Witness<'x>` is a zero-sized proof that code may mint brand `'x`. Only
//!   this module can make one, for any `'x`, `'static` included.
//! - `Rebrand` is implemented field by field, as a derive would write it:
//!   `rebrand` rebuilds a value at another brand, and `restore` rebuilds it
//!   back from one. A token implements both by copying its id: an index
//!   carries no lifetime, so nothing is transmuted.
//! - The engine stores every value at brand `'static` (`T::Of<'static>`, which
//!   is `'static` and so fits in a `dyn Any`) and every closure behind a
//!   wrapper that restores its argument at the brand the closure was built
//!   with, before calling it. The wrapper names that brand but captures
//!   nothing branded, so it is `'static`. A closure built in one `mutate` is
//!   thus run in another at its own, old brand; nothing it sees is branded
//!   with the brand of the `mutate` it runs in, so nothing can mix.
//!
//! Values leave `mutate` as an `Anchored<T>`, with the brand erased and a
//! guard that roots its tokens; `open` rebrands it to the current `mutate`.
//! `RemoteIo` queues sends, transactions, listeners and anchors from any
//! thread for the next `pump`, as in RFD 6.
//!
//! What runs: inputs, `map`, `filter_map`, `map_to`, `with`, `snapshot`,
//! `hold`, `accumulate`, `accumulate_mut`, `map_cell`, `node`, `share`,
//! `unzip`, listeners, anchors, collection, the handles. What is built for
//! its types only: `construct` stores its closure behind the same kind of
//! wrapper, which type checks, but never runs it; the switches and a loop's
//! evaluation order are not modelled.
//!
//! Two `--cfg` flags change the design, for the leak route (question 3):
//! `bough_split` moves `anchor`, `open` and the other I/O methods from the
//! `Build` that graph code gets to the `Mutate` only a `mutate` gets, and
//! `bough_auto` makes every graph closure require a nightly auto trait,
//! `GraphSafe`, that `Anchored` opts out of.
#![forbid(unsafe_code)]
#![allow(dead_code)]

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// Invariant in `'g`, so two brands never unify.
type Brand<'g> = PhantomData<fn(&'g ()) -> &'g ()>;

// ---------------------------------------------------------------------------
// The brand and how it changes

/// A proof that brand `'x` may be minted. Its field is private, so only this
/// module makes one; user code meets it only as the argument of its own
/// `Rebrand` impl, at a brand it can't name.
#[derive(Clone, Copy)]
pub struct Witness<'x> {
    brand: Brand<'x>,
}

impl<'x> Witness<'x> {
    fn conjure() -> Self {
        Witness { brand: PhantomData }
    }
}

/// The brand values are stored at.
fn erased() -> Witness<'static> {
    Witness::conjure()
}

/// The type a value has under another brand (gc-arena's `Rootable`), and how
/// to rebuild it there. A derive writes it next to `Trace`: `Of` renames the
/// brand, `rebrand` and `restore` go field by field.
pub trait Rebrand: Sized {
    type Of<'x>: 'x;

    /// This value at brand `'x`.
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x>;

    /// This value, rebuilt from its copy at brand `'x`.
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self;

    /// A borrow of the stored copy, for a type with no brand in it, whose
    /// `Of<'static>` is itself. A derive writes `Some(from)` for a type with
    /// no lifetime or type parameter; anything else is restored, a copy.
    fn view<'a>(from: &'a Self::Of<'static>) -> Option<&'a Self> {
        let _ = from;
        None
    }
}

/// A brand-free type: its own `Of`, cloned, and viewed in place.
macro_rules! plain {
    ($($t:ty),*) => {$(
        impl Rebrand for $t {
            type Of<'x> = $t;
            fn rebrand<'x>(&self, _: Witness<'x>) -> $t {
                self.clone()
            }
            fn restore<'x>(from: &$t, _: Witness<'x>) -> $t {
                from.clone()
            }
            fn view(from: &$t) -> Option<&$t> {
                Some(from)
            }
        }

        impl Trace for $t {
            fn trace(&self, _: &mut Tracer) {}
        }
    )*};
}

plain!((), bool, u32, u64, i64, String);

impl<A: Rebrand, B: Rebrand> Rebrand for (A, B) {
    type Of<'x> = (A::Of<'x>, B::Of<'x>);
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        (self.0.rebrand(to), self.1.rebrand(to))
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        (A::restore(&from.0, at), B::restore(&from.1, at))
    }
}

impl<A: Rebrand, B: Rebrand, C: Rebrand> Rebrand for (A, B, C) {
    type Of<'x> = (A::Of<'x>, B::Of<'x>, C::Of<'x>);
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        (self.0.rebrand(to), self.1.rebrand(to), self.2.rebrand(to))
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        (
            A::restore(&from.0, at),
            B::restore(&from.1, at),
            C::restore(&from.2, at),
        )
    }
}

impl<T: Rebrand> Rebrand for Vec<T> {
    type Of<'x> = Vec<T::Of<'x>>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        self.iter().map(|t| t.rebrand(to)).collect()
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        from.iter().map(|t| T::restore(t, at)).collect()
    }
}

impl<T: Rebrand> Rebrand for Option<T> {
    type Of<'x> = Option<T::Of<'x>>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        self.as_ref().map(|t| t.rebrand(to))
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        from.as_ref().map(|t| T::restore(t, at))
    }
}

/// A foreign value that holds no token: traced as nothing, and cloned across
/// brands. The orphan rules make it the only way to put another crate's
/// type in a stream, as it is already the only way to hold one.
#[derive(Clone, Debug, Default)]
pub struct Leaf<T>(pub T);

impl<T> Deref for Leaf<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone + 'static> Rebrand for Leaf<T> {
    type Of<'x> = Leaf<T>;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Leaf<T> {
        self.clone()
    }
    fn restore<'x>(from: &Leaf<T>, _: Witness<'x>) -> Leaf<T> {
        from.clone()
    }
    fn view(from: &Leaf<T>) -> Option<&Leaf<T>> {
        Some(from)
    }
}

impl<T> Trace for Leaf<T> {
    fn trace(&self, _: &mut Tracer) {}
}

/// The engine's two conversions: a stored value restored at the brand of the
/// code that asked, and a value stored at `'static`.
fn load<T: Rebrand>(stored: &dyn Any) -> T {
    let stored = stored
        .downcast_ref::<T::Of<'static>>()
        .expect("a stored value has the type its node was built with");
    T::restore(stored, erased())
}

fn store<T: Rebrand>(value: &T) -> Rc<dyn Any> {
    Rc::new(value.rebrand(erased()))
}

/// Call `f` on a stored value, borrowing it in place when its type has no
/// brand, restoring a copy when it has one.
fn with_loaded<T: Rebrand, R>(stored: &dyn Any, f: impl FnOnce(&T) -> R) -> R {
    let typed = stored
        .downcast_ref::<T::Of<'static>>()
        .expect("a stored value has the type its node was built with");
    match T::view(typed) {
        Some(value) => f(value),
        None => f(&T::restore(typed, erased())),
    }
}

// ---------------------------------------------------------------------------
// Tracing

/// What a `Trace` impl reports: the tokens a value holds.
pub struct Tracer {
    found: Vec<Id>,
}

pub trait Trace {
    fn trace(&self, tracer: &mut Tracer);
}

fn traced<T: Trace + ?Sized>(value: &T) -> Vec<Id> {
    let mut tracer = Tracer { found: Vec::new() };
    value.trace(&mut tracer);
    tracer.found
}

impl<A: Trace, B: Trace> Trace for (A, B) {
    fn trace(&self, tracer: &mut Tracer) {
        self.0.trace(tracer);
        self.1.trace(tracer);
    }
}

impl<A: Trace, B: Trace, C: Trace> Trace for (A, B, C) {
    fn trace(&self, tracer: &mut Tracer) {
        self.0.trace(tracer);
        self.1.trace(tracer);
        self.2.trace(tracer);
    }
}

impl<T: Trace> Trace for Vec<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.iter().for_each(|t| t.trace(tracer));
    }
}

impl<T: Trace> Trace for Option<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.iter().for_each(|t| t.trace(tracer));
    }
}

impl<K, V: Trace> Trace for HashMap<K, V> {
    fn trace(&self, tracer: &mut Tracer) {
        self.values().for_each(|v| v.trace(tracer));
    }
}

// ---------------------------------------------------------------------------
// The auto trait of design `auto`

/// Behind a macro so that stable `rustc` never parses the nightly syntax.
#[cfg(bough_auto)]
macro_rules! graph_safe {
    () => {
        pub auto trait GraphSafe {}
        impl<T> !GraphSafe for Anchored<T> {}
    };
}

#[cfg(bough_auto)]
graph_safe!();

/// On stable, every type is `GraphSafe` and the bound does nothing.
#[cfg(not(bough_auto))]
pub trait GraphSafe {}

#[cfg(not(bough_auto))]
impl<T: ?Sized> GraphSafe for T {}

// ---------------------------------------------------------------------------
// Tokens

/// Public so that sealed signatures may name it; its fields are private.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Id {
    index: u32,
    generation: u32,
    graph: u32,
}

/// Why a token can't be used.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Its node was collected.
    Stale,
    /// It names a node of another runtime.
    ForeignGraph,
}

macro_rules! token {
    ($name:ident) => {
        pub struct $name<'g, A> {
            id: Id,
            event: PhantomData<fn() -> A>,
            brand: Brand<'g>,
        }

        impl<'g, A> Clone for $name<'g, A> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<'g, A> Copy for $name<'g, A> {}

        impl<'g, A> Trace for $name<'g, A> {
            fn trace(&self, tracer: &mut Tracer) {
                tracer.found.push(self.id);
            }
        }

        /// A token rebrands by copying its id: the whole of the safe route.
        impl<'g, A: Rebrand> Rebrand for $name<'g, A> {
            type Of<'x> = $name<'x, A::Of<'x>>;
            fn rebrand<'x>(&self, _: Witness<'x>) -> Self::Of<'x> {
                $name::at(self.id)
            }
            fn restore<'x>(from: &Self::Of<'x>, _: Witness<'x>) -> Self {
                $name::at(from.id)
            }
        }

        impl<'g, A> $name<'g, A> {
            fn at(id: Id) -> Self {
                $name {
                    id,
                    event: PhantomData,
                    brand: PhantomData,
                }
            }

            /// The arena slot it names, printed to show which node it is.
            pub fn index(&self) -> u32 {
                self.id.index
            }
        }
    };
}

token!(Cell);
token!(Shared);
token!(Input);

/// A pipe is the fused chain of a linear stream's adapters, from its source
/// node's stored event to its own, both at brand `'static`.
type Pipe = Rc<dyn Fn(&View, &dyn Any) -> Option<Rc<dyn Any>>>;

/// A linear stream: a source node and the adapters not yet materialized.
pub struct Stream<'g, A> {
    source: Id,
    pipe: Option<Pipe>,
    env: Vec<Id>,
    event: PhantomData<fn() -> A>,
    brand: Brand<'g>,
}

impl<'g, A> Clone for Stream<'g, A> {
    fn clone(&self) -> Self {
        Stream {
            source: self.source,
            pipe: self.pipe.clone(),
            env: self.env.clone(),
            event: PhantomData,
            brand: PhantomData,
        }
    }
}

impl<'g, A> Stream<'g, A> {
    fn at(source: Id) -> Self {
        Stream {
            source,
            pipe: None,
            env: Vec::new(),
            event: PhantomData,
            brand: PhantomData,
        }
    }
}

impl<'g, A> Trace for Stream<'g, A> {
    fn trace(&self, tracer: &mut Tracer) {
        tracer.found.push(self.source);
        tracer.found.extend(&self.env);
    }
}

impl<'g, A: Rebrand> Rebrand for Stream<'g, A> {
    type Of<'x> = Stream<'x, A::Of<'x>>;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Self::Of<'x> {
        let Stream {
            source, pipe, env, ..
        } = self.clone();
        Stream {
            source,
            pipe,
            env,
            event: PhantomData,
            brand: PhantomData,
        }
    }
    fn restore<'x>(from: &Self::Of<'x>, _: Witness<'x>) -> Self {
        Stream {
            source: from.source,
            pipe: from.pipe.clone(),
            env: from.env.clone(),
            event: PhantomData,
            brand: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------------
// Guards

/// A root with the brand erased, `'static`, `Send` when its value is, so I/O
/// code keeps it across units and threads. It doesn't deref: the value is
/// reached by opening it in a `mutate`, at that call's brand. It isn't
/// `Trace`, so a derived `Trace` on a type that holds one doesn't compile.
pub struct Anchored<T> {
    value: T,
    guard: Arc<()>,
    graph: u32,
}

impl<T: Clone> Clone for Anchored<T> {
    fn clone(&self) -> Self {
        Anchored {
            value: self.value.clone(),
            guard: self.guard.clone(),
            graph: self.graph,
        }
    }
}

impl<T: Clone + 'static> Rebrand for Anchored<T> {
    type Of<'x> = Anchored<T>;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Anchored<T> {
        self.clone()
    }
    fn restore<'x>(from: &Anchored<T>, _: Witness<'x>) -> Anchored<T> {
        from.clone()
    }
}

fn anchored<T: Trace + Rebrand>(value: &T, graph: u32) -> (Anchored<T::Of<'static>>, Root) {
    let guard = Arc::new(());
    let root = Root {
        alive: Arc::downgrade(&guard),
        ids: traced(value),
    };
    let anchored = Anchored {
        value: value.rebrand(erased()),
        guard,
        graph,
    };
    (anchored, root)
}

/// A listener's guard. Dropping it unlistens; `keep` leaves it live.
pub struct Listener {
    alive: Arc<()>,
}

impl Listener {
    pub fn keep(self) {
        std::mem::forget(self.alive);
    }
}

struct Root {
    alive: Weak<()>,
    ids: Vec<Id>,
}

struct Listening {
    node: Id,
    alive: Weak<()>,
    call: Box<dyn FnMut(&dyn Any)>,
}

struct RemoteListening {
    node: Id,
    alive: Weak<()>,
    call: Box<dyn FnMut(&dyn Any) + Send>,
}

// ---------------------------------------------------------------------------
// The arena

/// What evaluation reads: this unit's firings so far, and the cells' values
/// as they were before it.
pub struct View<'a> {
    nodes: &'a [Node],
    firing: &'a [Option<Rc<dyn Any>>],
}

impl View<'_> {
    fn fired(&self, id: Id) -> Option<&Rc<dyn Any>> {
        self.firing.get(id.index as usize)?.as_ref()
    }

    fn value(&self, id: Id) -> &dyn Any {
        match &self.nodes[id.index as usize].kind {
            Kind::Cell { value, .. } => &**value,
            _ => panic!("a cell token names a cell"),
        }
    }
}

type Next = Box<dyn Fn(&View, &dyn Any) -> Option<Rc<dyn Any>>>;
type TraceFn = Box<dyn Fn(&dyn Any, &mut Vec<Id>)>;

enum Kind {
    Free,
    Input,
    /// A materialized chain: fires with its pipe's output when its source
    /// fires.
    Stream {
        source: Id,
        pipe: Option<Pipe>,
    },
    /// A cell; `next` computes its new value from this unit's firings and its
    /// current value, and `trace` finds the tokens its value holds.
    Cell {
        value: Rc<dyn Any>,
        next: Option<Next>,
        trace: TraceFn,
    },
    /// Built for its types only; a `construct` keeps its wrapped closure here.
    Inert {
        stored: Option<Box<dyn Any>>,
    },
}

struct Node {
    generation: u32,
    deps: Vec<Id>,
    kind: Kind,
}

static GRAPHS: AtomicU32 = AtomicU32::new(1);

#[derive(Default)]
struct Core {
    graph: u32,
    nodes: Vec<Node>,
    free: VecDeque<u32>,
    roots: Vec<Root>,
    listeners: Vec<Listening>,
    /// The sends of the current `mutate`, run as one unit at its end.
    sends: Vec<(Id, Rc<dyn Any>)>,
    /// Stale or foreign sends dropped at a unit, as a release build counts.
    dropped_sends: u32,
}

impl Core {
    fn mint(&mut self, deps: Vec<Id>, kind: Kind) -> Id {
        let node = |generation| Node {
            generation,
            deps,
            kind,
        };
        let index = match self.free.pop_front() {
            Some(index) => {
                let slot = &mut self.nodes[index as usize];
                *slot = node(slot.generation);
                index
            }
            None => {
                self.nodes.push(node(0));
                self.nodes.len() as u32 - 1
            }
        };
        Id {
            index,
            generation: self.nodes[index as usize].generation,
            graph: self.graph,
        }
    }

    fn check(&self, id: Id) -> Result<(), Error> {
        if id.graph != self.graph {
            return Err(Error::ForeignGraph);
        }
        let node = &self.nodes[id.index as usize];
        if node.generation != id.generation || matches!(node.kind, Kind::Free) {
            return Err(Error::Stale);
        }
        Ok(())
    }

    fn live(&self, id: Id) -> Id {
        if let Err(error) = self.check(id) {
            panic!("a token used in a build is {error:?}");
        }
        id
    }

    fn run_unit(&mut self, sends: Vec<(Id, Rc<dyn Any>)>) {
        let mut firing: Vec<Option<Rc<dyn Any>>> = vec![None; self.nodes.len()];
        for (id, value) in sends {
            match self.check(id) {
                Ok(()) => firing[id.index as usize] = Some(value),
                Err(_) => self.dropped_sends += 1,
            }
        }
        // Nodes are evaluated in index order, which is build order: a toy
        // stand-in for rank, wrong only for a loop.
        let mut commits = Vec::new();
        for i in 0..self.nodes.len() {
            let view = View {
                nodes: &self.nodes,
                firing: &firing,
            };
            let out = match &self.nodes[i].kind {
                Kind::Stream { source, pipe } => view.fired(*source).and_then(|x| match pipe {
                    Some(pipe) => pipe(&view, &**x),
                    None => Some(x.clone()),
                }),
                Kind::Cell {
                    value,
                    next: Some(next),
                    ..
                } => next(&view, &**value),
                _ => None,
            };
            if let (Some(value), Kind::Cell { .. }) = (&out, &self.nodes[i].kind) {
                commits.push((i, value.clone()));
            }
            if out.is_some() {
                firing[i] = out;
            }
        }
        for (i, new) in commits {
            if let Kind::Cell { value, .. } = &mut self.nodes[i].kind {
                *value = new;
            }
        }
        for listener in &mut self.listeners {
            if listener.alive.strong_count() == 0 {
                continue;
            }
            if let Some(Some(value)) = firing.get(listener.node.index as usize) {
                (listener.call)(&**value);
            }
        }
    }

    /// Mark from every live guard, free the rest, and return how many.
    fn collect(&mut self) -> usize {
        self.roots.retain(|root| root.alive.strong_count() > 0);
        self.listeners
            .retain(|listener| listener.alive.strong_count() > 0);
        let mut marked = vec![false; self.nodes.len()];
        let mut stack: Vec<Id> = self.roots.iter().flat_map(|r| r.ids.clone()).collect();
        stack.extend(self.listeners.iter().map(|l| l.node));
        while let Some(id) = stack.pop() {
            if self.check(id).is_err() || marked[id.index as usize] {
                continue;
            }
            marked[id.index as usize] = true;
            let node = &self.nodes[id.index as usize];
            stack.extend(&node.deps);
            if let Kind::Cell { value, trace, .. } = &node.kind {
                trace(&**value, &mut stack);
            }
        }
        let mut freed = 0;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if !marked[i] && !matches!(node.kind, Kind::Free) {
                node.kind = Kind::Free;
                node.deps.clear();
                node.generation += 1;
                self.free.push_back(i as u32);
                freed += 1;
            }
        }
        freed
    }

    fn live_nodes(&self) -> usize {
        let free = self
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, Kind::Free))
            .count();
        self.nodes.len() - free
    }

    fn cell<S: Trace + Rebrand>(&mut self, deps: Vec<Id>, init: &S, next: Option<Next>) -> Id {
        let trace: TraceFn = Box::new(|value: &dyn Any, found: &mut Vec<Id>| {
            found.extend(traced(&load::<S>(value)))
        });
        self.mint(
            deps,
            Kind::Cell {
                value: store(init),
                next,
                trace,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Build, Mutate, Runtime

/// What graph code gets: the top level of a `mutate`, and a `construct`'s
/// closure when it runs. Owns the arena for the length of the call.
pub struct Build<'g> {
    core: Core,
    brand: Brand<'g>,
}

/// What only a `mutate` (or a queued call, which runs as one) gets. It
/// derefs to `Build`, so a helper taking `&mut Build<'g>` takes it too.
pub struct Mutate<'g> {
    build: Build<'g>,
}

impl<'g> Deref for Mutate<'g> {
    type Target = Build<'g>;
    fn deref(&self) -> &Build<'g> {
        &self.build
    }
}

impl<'g> DerefMut for Mutate<'g> {
    fn deref_mut(&mut self) -> &mut Build<'g> {
        &mut self.build
    }
}

impl<'g> Build<'g> {
    pub fn input<A: Rebrand>(&mut self) -> (Stream<'g, A>, Input<'g, A>) {
        let id = self.core.mint(Vec::new(), Kind::Input);
        (Stream::at(id), Input::at(id))
    }

    /// A loop is built for its types and its reachability; evaluation in
    /// index order doesn't honour it.
    pub fn stream_loop<A: Rebrand>(&mut self) -> (Stream<'g, A>, StreamLoop<'g, A>) {
        let id = self.core.mint(Vec::new(), Kind::Inert { stored: None });
        (
            Stream::at(id),
            StreamLoop {
                id,
                event: PhantomData,
                brand: PhantomData,
            },
        )
    }
}

/// The I/O methods: `anchor`, `open`, `send`, the listens, `sample`,
/// `collect`. On `Build` in the default design, as in the earlier probe; on
/// `Mutate` only in design `split`, so a `construct` closure lacks them.
macro_rules! io_methods {
    () => {
        pub fn anchor<T: Trace + Rebrand>(&mut self, value: T) -> Anchored<T::Of<'static>> {
            let (anchored, root) = anchored(&value, self.core.graph);
            self.core.roots.push(root);
            anchored
        }

        /// The anchored value at this `mutate`'s brand: a copy of its ids.
        pub fn open<T: Rebrand>(&self, anchored: &Anchored<T>) -> T::Of<'g> {
            anchored.value.rebrand(Witness::conjure())
        }

        /// Queued for the end of this `mutate`, where its sends run as one unit.
        pub fn send<A: Rebrand>(&mut self, input: Input<'g, A>, value: A) {
            self.core.sends.push((input.id, store(&value)));
        }

        pub fn sample<A: Rebrand>(&self, cell: Cell<'g, A>) -> Result<A, Error> {
            self.core.check(cell.id)?;
            Ok(load(self.core.nodes[cell.id.index as usize].value()))
        }

        pub fn listen<S: Source<'g>>(
            &mut self,
            source: S,
            mut f: impl FnMut(S::Event) + 'static,
        ) -> Listener {
            let node = source.node(self).source;
            self.register(node, Box::new(move |x: &dyn Any| f(load(x))))
        }

        pub fn listen_cell<A: Rebrand>(
            &mut self,
            cell: Cell<'g, A>,
            mut f: impl FnMut(&A) + 'static,
        ) -> Listener {
            let node = self.core.live(cell.id);
            self.register(node, Box::new(move |x: &dyn Any| with_loaded(x, |a| f(a))))
        }

        fn register(&mut self, node: Id, call: Box<dyn FnMut(&dyn Any)>) -> Listener {
            let alive = Arc::new(());
            self.core.listeners.push(Listening {
                node,
                alive: Arc::downgrade(&alive),
                call,
            });
            Listener { alive }
        }

        pub fn collect(&mut self) -> usize {
            self.core.collect()
        }

        pub fn live_nodes(&self) -> usize {
            self.core.live_nodes()
        }
    };
}

#[cfg(not(bough_split))]
impl<'g> Build<'g> {
    io_methods!();
}

#[cfg(bough_split)]
impl<'g> Mutate<'g> {
    io_methods!();
}

impl Node {
    fn value(&self) -> &dyn Any {
        match &self.kind {
            Kind::Cell { value, .. } => &**value,
            _ => panic!("sampling a node that isn't an evaluated cell"),
        }
    }
}

pub struct StreamLoop<'g, A> {
    id: Id,
    event: PhantomData<fn() -> A>,
    brand: Brand<'g>,
}

impl<'g, A: Rebrand> StreamLoop<'g, A> {
    pub fn close(self, b: &mut Build<'g>, source: impl Source<'g, Event = A>) {
        let (source, pipe, mut env) = source.parts();
        env.push(source);
        let node = &mut b.core.nodes[self.id.index as usize];
        node.deps = env;
        node.kind = Kind::Stream { source, pipe };
    }
}

type Queued = Box<dyn for<'g> FnOnce(&mut Mutate<'g>) + Send>;

#[derive(Default)]
struct Inbox {
    calls: Vec<Queued>,
    roots: Vec<Root>,
    listeners: Vec<RemoteListening>,
}

pub struct Runtime {
    core: Core,
    inbox: Arc<Mutex<Inbox>>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Runtime {
            core: Core {
                graph: GRAPHS.fetch_add(1, Ordering::Relaxed),
                ..Core::default()
            },
            inbox: Arc::default(),
        }
    }

    /// Build and I/O both go through here. `'g` is fresh for each call, so
    /// nothing branded leaves it: what must, leaves anchored.
    pub fn mutate<R: 'static>(&mut self, f: impl for<'g> FnOnce(&mut Mutate<'g>) -> R) -> R {
        let mut m = Mutate {
            build: Build {
                core: std::mem::take(&mut self.core),
                brand: PhantomData,
            },
        };
        let r = f(&mut m);
        self.core = m.build.core;
        let sends = std::mem::take(&mut self.core.sends);
        if !sends.is_empty() {
            self.core.run_unit(sends);
        }
        r
    }

    /// A send needs no brand: the anchored input's id is enough.
    pub fn send<A: 'static>(&mut self, input: &Anchored<Input<'static, A>>, value: A) {
        self.core.run_unit(vec![(input.value.id, Rc::new(value))]);
    }

    pub fn remote_io(&self) -> RemoteIo {
        RemoteIo {
            inbox: self.inbox.clone(),
            graph: self.core.graph,
        }
    }

    /// Register what the handles queued, then run their calls in order, each
    /// as its own `mutate`, so each is one unit. Returns how many ran.
    pub fn pump(&mut self) -> usize {
        let Inbox {
            calls,
            roots,
            listeners,
        } = std::mem::take(&mut *self.inbox.lock().expect("the inbox lock"));
        self.core.roots.extend(roots);
        self.core
            .listeners
            .extend(listeners.into_iter().map(|l| Listening {
                node: l.node,
                alive: l.alive,
                call: l.call,
            }));
        let ran = calls.len();
        for call in calls {
            self.mutate(|m| call(m));
        }
        ran
    }

    pub fn collect(&mut self) -> usize {
        self.core.collect()
    }

    pub fn live_nodes(&self) -> usize {
        self.core.live_nodes()
    }

    pub fn dropped_sends(&self) -> u32 {
        self.core.dropped_sends
    }
}

/// RFD 6's handle for I/O code on any thread: every call queues for the
/// driver's next `pump`. It names nodes only through `Anchored` values, or
/// through values at a brand it doesn't care about (`anchor`).
#[derive(Clone)]
pub struct RemoteIo {
    inbox: Arc<Mutex<Inbox>>,
    graph: u32,
}

impl RemoteIo {
    fn queue(&self, call: Queued) {
        self.inbox.lock().expect("the inbox lock").calls.push(call);
    }

    /// No closure is needed for a send: the anchored input's id, and a value
    /// already at brand `'static`, are what the unit stores.
    pub fn send<A: Send + 'static>(&self, input: &Anchored<Input<'static, A>>, value: A) {
        let id = input.value.id;
        self.queue(Box::new(move |m: &mut Mutate<'_>| {
            m.core.sends.push((id, Rc::new(value)))
        }));
    }

    /// Simultaneous sends: a closure the driver runs as a `mutate`, which
    /// opens what the I/O code anchored.
    pub fn transaction(&self, f: impl for<'g> FnOnce(&mut Mutate<'g>) + Send + 'static) {
        self.queue(Box::new(f));
    }

    /// A listener on an anchored stream gets each event at a brand of its
    /// own, fresh for every call, so it can't keep a token it's handed.
    pub fn listen<A: Rebrand + 'static>(
        &self,
        shared: &Anchored<Shared<'static, A>>,
        mut f: impl for<'x> FnMut(A::Of<'x>) + Send + 'static,
    ) -> Listener {
        let alive = Arc::new(());
        let call = Box::new(move |x: &dyn Any| {
            let stored = x.downcast_ref::<A>().expect("the anchored event type");
            f(stored.rebrand(Witness::conjure()))
        });
        self.inbox
            .lock()
            .expect("the inbox lock")
            .listeners
            .push(RemoteListening {
                node: shared.value.id,
                alive: Arc::downgrade(&alive),
                call,
            });
        Listener { alive }
    }

    /// Anchor a value of any brand, as a listener handed a row does: the row
    /// is traced now and its tokens wait in the queue until the pump roots
    /// them.
    pub fn anchor<T: Trace + Rebrand>(&self, value: T) -> Anchored<T::Of<'static>> {
        let (anchored, root) = anchored(&value, self.graph);
        self.inbox.lock().expect("the inbox lock").roots.push(root);
        anchored
    }
}

// ---------------------------------------------------------------------------
// Adapters and materializers

/// What a source is made of. The trait is in a private module, so only this
/// module makes sources.
mod sealed {
    pub trait Parts {
        fn parts(self) -> (super::Id, Option<super::Pipe>, Vec<super::Id>);
    }
}

/// One more adapter on a chain. The new pipe restores the event at brand
/// `'g`, where `step` was written, and stores what it returns at `'static`;
/// it captures `step` and the old pipe, neither of them branded, so it is
/// `'static` although its body names `'g`.
fn then<'g, S: Source<'g>, B: Rebrand>(
    source: S,
    step: impl Fn(&View, S::Event) -> Option<B> + 'static,
) -> Stream<'g, B> {
    let (source, pipe, env) = source.parts();
    let pipe: Pipe = Rc::new(move |view: &View, x: &dyn Any| {
        let event: S::Event = match &pipe {
            Some(pipe) => load(&*pipe(view, x)?),
            None => load(x),
        };
        step(view, event).map(|b| store(&b))
    });
    Stream {
        source,
        pipe: Some(pipe),
        env,
        event: PhantomData,
        brand: PhantomData,
    }
}

/// Every closure an adapter or materializer stores is `'static`, as in RFD
/// 3, and `GraphSafe`, which is every type except in design `auto`.
pub trait Source<'g>: sealed::Parts + Sized {
    type Event: Rebrand;

    /// The one way a closure gets a token it didn't receive: the environment
    /// travels with each event, as data the chain traces (spike F93, F94).
    fn with<E: Trace + Rebrand>(self, env: E) -> Stream<'g, (E, Self::Event)> {
        let ids = traced(&env);
        let env = env.rebrand(erased());
        let mut out = then(self, move |_, event| {
            Some((E::restore(&env, erased()), event))
        });
        out.env.extend(ids);
        out
    }

    fn map<B: Rebrand, F: Fn(Self::Event) -> B + GraphSafe + 'static>(self, f: F) -> Stream<'g, B> {
        then(self, move |_, event| Some(f(event)))
    }

    fn filter_map<B: Rebrand, F: Fn(Self::Event) -> Option<B> + GraphSafe + 'static>(
        self,
        f: F,
    ) -> Stream<'g, B> {
        then(self, move |_, event| f(event))
    }

    fn map_to<B: Trace + Rebrand>(self, value: B) -> Stream<'g, B> {
        let ids = traced(&value);
        let value = value.rebrand(erased());
        let mut out = then(self, move |_, _| Some(B::restore(&value, erased())));
        out.env.extend(ids);
        out
    }

    fn snapshot<C: Rebrand, B: Rebrand, F: Fn(Self::Event, &C) -> B + GraphSafe + 'static>(
        self,
        cell: Cell<'g, C>,
        f: F,
    ) -> Stream<'g, B> {
        let id = cell.id;
        let mut out = then(self, move |view, event| {
            Some(with_loaded(view.value(id), |c| f(event, c)))
        });
        out.env.push(id);
        out
    }

    /// Materialize the chain as a node of its own.
    fn node(self, b: &mut Build<'g>) -> Stream<'g, Self::Event> {
        let core = &mut b.core;
        let (source, pipe, mut env) = self.parts();
        let source = core.live(source);
        env.push(source);
        Stream::at(core.mint(env, Kind::Stream { source, pipe }))
    }

    fn share(self, b: &mut Build<'g>) -> Shared<'g, Self::Event> {
        Shared::at(self.node(b).source)
    }

    fn hold(self, b: &mut Build<'g>, init: Self::Event) -> Cell<'g, Self::Event>
    where
        Self::Event: Trace,
    {
        let from = self.node(b).source;
        let next: Next = Box::new(move |view: &View, _: &dyn Any| view.fired(from).cloned());
        Cell::at(b.core.cell(vec![from], &init, Some(next)))
    }

    fn accumulate<S: Trace + Rebrand, F: Fn(Self::Event, &S) -> S + GraphSafe + 'static>(
        self,
        b: &mut Build<'g>,
        init: S,
        f: F,
    ) -> Cell<'g, S> {
        let from = self.node(b).source;
        let next: Next = Box::new(move |view: &View, state: &dyn Any| {
            let event: Self::Event = load(&**view.fired(from)?);
            Some(store(&with_loaded(state, |s| f(event, s))))
        });
        Cell::at(b.core.cell(vec![from], &init, Some(next)))
    }

    /// Safe erasure restores a copy of the state, changes it and stores it
    /// back, per event. A real engine keeping the state mutable in its node
    /// could change it in place for a type with no brand, as `view` borrows
    /// one; a state that holds tokens is always copied.
    fn accumulate_mut<S: Trace + Rebrand, F: Fn(Self::Event, &mut S) + GraphSafe + 'static>(
        self,
        b: &mut Build<'g>,
        init: S,
        f: F,
    ) -> Cell<'g, S> {
        let from = self.node(b).source;
        let next: Next = Box::new(move |view: &View, state: &dyn Any| {
            let event: Self::Event = load(&**view.fired(from)?);
            let mut state: S = load(state);
            f(event, &mut state);
            Some(store(&state))
        });
        Cell::at(b.core.cell(vec![from], &init, Some(next)))
    }

    /// Built for its types: the closure is stored behind a wrapper that runs
    /// it at its own brand, given the arena, and stores what it returns. The
    /// wrapper type checks without `unsafe`; the toy never calls it.
    fn construct<B: Rebrand, F: Fn(&mut Build<'g>, Self::Event) -> B + GraphSafe + 'static>(
        self,
        b: &mut Build<'g>,
        f: F,
    ) -> Stream<'g, B> {
        let from = self.node(b).source;
        let wrapped: Box<dyn Fn(Core, &dyn Any) -> (Core, Rc<dyn Any>)> =
            Box::new(move |core: Core, x: &dyn Any| {
                let mut build = Build::<'g> {
                    core,
                    brand: PhantomData,
                };
                let out = f(&mut build, load(x));
                (build.core, store(&out))
            });
        let stored: Box<dyn Any> = Box::new(wrapped);
        Stream::at(b.core.mint(
            vec![from],
            Kind::Inert {
                stored: Some(stored),
            },
        ))
    }
}

impl<'g, A: Rebrand> Source<'g> for Stream<'g, A> {
    type Event = A;
}

impl<'g, A> sealed::Parts for Stream<'g, A> {
    fn parts(self) -> (Id, Option<Pipe>, Vec<Id>) {
        (self.source, self.pipe, self.env)
    }
}

impl<'g, A: Rebrand> Source<'g> for Shared<'g, A> {
    type Event = A;
}

impl<'g, A> sealed::Parts for Shared<'g, A> {
    fn parts(self) -> (Id, Option<Pipe>, Vec<Id>) {
        (self.id, None, Vec::new())
    }
}

impl<'g, A: Rebrand, B: Rebrand> Stream<'g, (A, B)> {
    /// RFD 4's `unzip`: each half of a pair to a linear stream of its own.
    pub fn unzip(self, b: &mut Build<'g>) -> (Stream<'g, A>, Stream<'g, B>) {
        let both = Shared::<'g, (A, B)>::at(self.node(b).source);
        let left = then(both, |_, (a, _)| Some(a)).node(b);
        let right = then(both, |_, (_, b)| Some(b)).node(b);
        (left, right)
    }
}

impl<'g, A: Rebrand> Cell<'g, A> {
    pub fn map_cell<B: Rebrand + Trace, F: Fn(&A) -> B + GraphSafe + 'static>(
        self,
        b: &mut Build<'g>,
        f: F,
    ) -> Cell<'g, B> {
        let from = b.core.live(self.id);
        let init = with_loaded(b.core.nodes[from.index as usize].value(), |a| f(a));
        let next: Next = Box::new(move |view: &View, _: &dyn Any| {
            let fired = view.fired(from)?;
            Some(store(&with_loaded(&**fired, |a| f(a))))
        });
        Cell::at(b.core.cell(vec![from], &init, Some(next)))
    }
}

impl<'g, A: Rebrand> Cell<'g, Stream<'g, A>> {
    /// Built for its types only.
    pub fn switch_stream(self, b: &mut Build<'g>) -> Stream<'g, A> {
        let from = b.core.live(self.id);
        Stream::at(b.core.mint(vec![from], Kind::Inert { stored: None }))
    }
}

impl<'g, A: Rebrand> Cell<'g, Cell<'g, A>> {
    /// Built for its types only.
    pub fn switch_cell(self, b: &mut Build<'g>) -> Cell<'g, A> {
        let from = b.core.live(self.id);
        Cell::at(b.core.mint(vec![from], Kind::Inert { stored: None }))
    }
}

// ===========================================================================
// Borrowed views, for `rfd-0003-brand-erasure -- borrow`
//
// Everything above this line is `../api.rs`, byte for byte, and the binary
// checks it: the views need the runtime's private parts (a token's id, the
// stored values, `erased`), so they are appended to a copy of it rather than
// written against its public API.
//
// `sample` restores a copy of a stored value at the reader's brand, and
// `accumulate_mut` restores one, mutates it and rebrands it back. RFD 4's
// `sample` returns `&A` from a memo instead, and `accumulate_mut` hands `f` a
// `&mut S`. With a brand in the value neither is safe: the stored copy is an
// `A::Of<'static>`, and `&A` of it is gc-arena's cast. The views are the safe
// alternative the cost probes priced: a value built over the stored copy
// that holds each token at the reader's or writer's brand, as a copy of its
// id, and borrows everything else in place.

use std::cell::OnceCell;

/// A proof that the engine is lending its stored copy for `'a`. Only this
/// module makes one. `borrow` takes one so that no one else can call it on a
/// stored token and get it at any brand they like; it is scoped to the loan,
/// not `'static`, so an impl can't keep it past the call.
#[derive(Clone, Copy)]
pub struct Loan<'a> {
    brand: Brand<'a>,
}

impl<'a> Loan<'a> {
    fn conjure() -> Self {
        Loan { brand: PhantomData }
    }
}

/// The read view of a stored value, which a derive writes next to `Rebrand`
/// as a `FooRef<'a, 'g>`: tokens at the reader's brand `'g` (the brand of
/// `Self`), and everything else borrowed from the stored copy for `'a`.
///
/// `Ref<'a>` has no `where Self: 'a`: a view borrows the stored copy, which is
/// `'static`, never `Self`, so it needs no bound, and without one a closure
/// may take `T::Ref<'_>` for every `'_` (a higher-ranked bound over a GAT
/// with `Self: 'a` would demand `T: 'static`).
pub trait Borrow: Rebrand {
    type Ref<'a>;

    fn borrow<'a>(from: &'a Self::Of<'static>, at: Loan<'a>) -> Self::Ref<'a>;
}

/// The write view: tokens read and written at the writer's brand through a
/// `Place`, data borrowed mutably, a collection behind a `ListMut`. Nothing
/// in it hands out the stored `'static` copy.
pub trait BorrowMut: Borrow {
    type Mut<'a>;

    fn borrow_mut<'a>(from: &'a mut Self::Of<'static>, at: Loan<'a>) -> Self::Mut<'a>;
}

/// One stored value, whole, at brand `'g` (the brand of `T`): read it as a
/// copy, replace it, or view it. A token's write view is its `Place`; so is
/// the root a `Build::update` hands out, so that a whole value, an enum's
/// variant included, can be replaced.
pub struct Place<'a, T: Rebrand> {
    slot: &'a mut T::Of<'static>,
}

impl<'a, T: Rebrand> Place<'a, T> {
    /// A copy, restored at the writer's brand: for a token, its id.
    pub fn get(&self) -> T {
        T::restore(self.slot, erased())
    }

    pub fn set(&mut self, value: T) {
        *self.slot = value.rebrand(erased());
    }
}

impl<'a, T: Borrow> Place<'a, T> {
    pub fn get_ref(&self) -> T::Ref<'_> {
        T::borrow(self.slot, Loan::conjure())
    }
}

impl<'a, T: BorrowMut> Place<'a, T> {
    pub fn as_mut(&mut self) -> T::Mut<'_> {
        T::borrow_mut(self.slot, Loan::conjure())
    }

    pub fn into_mut(self) -> T::Mut<'a> {
        T::borrow_mut(self.slot, Loan::conjure())
    }
}

/// A brand-free type is its own stored copy: its views are references.
macro_rules! plain_views {
    ($($t:ty),*) => {$(
        impl Borrow for $t {
            type Ref<'a> = &'a $t;
            fn borrow<'a>(from: &'a $t, _: Loan<'a>) -> &'a $t {
                from
            }
        }

        impl BorrowMut for $t {
            type Mut<'a> = &'a mut $t;
            fn borrow_mut<'a>(from: &'a mut $t, _: Loan<'a>) -> &'a mut $t {
                from
            }
        }
    )*};
}

plain_views!((), bool, u32, u64, i64, String);

impl<T: Clone + 'static> Borrow for Leaf<T> {
    type Ref<'a> = &'a Leaf<T>;
    fn borrow<'a>(from: &'a Leaf<T>, _: Loan<'a>) -> &'a Leaf<T> {
        from
    }
}

impl<T: Clone + 'static> BorrowMut for Leaf<T> {
    type Mut<'a> = &'a mut Leaf<T>;
    fn borrow_mut<'a>(from: &'a mut Leaf<T>, _: Loan<'a>) -> &'a mut Leaf<T> {
        from
    }
}

impl<A: Borrow, B: Borrow> Borrow for (A, B) {
    type Ref<'a> = (A::Ref<'a>, B::Ref<'a>);
    fn borrow<'a>(from: &'a Self::Of<'static>, at: Loan<'a>) -> Self::Ref<'a> {
        (A::borrow(&from.0, at), B::borrow(&from.1, at))
    }
}

impl<A: BorrowMut, B: BorrowMut> BorrowMut for (A, B) {
    type Mut<'a> = (A::Mut<'a>, B::Mut<'a>);
    fn borrow_mut<'a>(from: &'a mut Self::Of<'static>, at: Loan<'a>) -> Self::Mut<'a> {
        let (a, b) = from;
        (A::borrow_mut(a, at), B::borrow_mut(b, at))
    }
}

/// A token's read view is itself at the reader's brand, rebuilt from its id;
/// its write view is its `Place`.
macro_rules! token_views {
    ($($name:ident),*) => {$(
        impl<'g, A: Rebrand> Borrow for $name<'g, A> {
            type Ref<'a> = $name<'g, A>;
            fn borrow<'a>(from: &'a $name<'static, A::Of<'static>>, _: Loan<'a>) -> Self {
                $name::at(from.id)
            }
        }

        impl<'g, A: Rebrand> BorrowMut for $name<'g, A> {
            type Mut<'a> = Place<'a, Self>;
            fn borrow_mut<'a>(
                from: &'a mut $name<'static, A::Of<'static>>,
                _: Loan<'a>,
            ) -> Place<'a, Self> {
                Place { slot: from }
            }
        }
    )*};
}

token_views!(Cell, Shared, Input);

impl<'g, A: Rebrand> Borrow for Stream<'g, A> {
    type Ref<'a> = Stream<'g, A>;
    fn borrow<'a>(from: &'a Stream<'static, A::Of<'static>>, _: Loan<'a>) -> Self {
        Stream::restore(from, erased())
    }
}

impl<'g, A: Rebrand> BorrowMut for Stream<'g, A> {
    type Mut<'a> = Place<'a, Self>;
    fn borrow_mut<'a>(
        from: &'a mut Stream<'static, A::Of<'static>>,
        _: Loan<'a>,
    ) -> Place<'a, Self> {
        Place { slot: from }
    }
}

impl<T: Clone + 'static> Borrow for Anchored<T> {
    type Ref<'a> = &'a Anchored<T>;
    fn borrow<'a>(from: &'a Anchored<T>, _: Loan<'a>) -> &'a Anchored<T> {
        from
    }
}

impl<T: Borrow> Borrow for Option<T> {
    type Ref<'a> = Option<T::Ref<'a>>;
    fn borrow<'a>(from: &'a Option<T::Of<'static>>, at: Loan<'a>) -> Self::Ref<'a> {
        from.as_ref().map(|t| T::borrow(t, at))
    }
}

/// An `Option` in place: its element viewed, or the whole replaced.
pub struct OptionMut<'a, T: Rebrand> {
    slot: &'a mut Option<T::Of<'static>>,
}

impl<'a, T: BorrowMut> OptionMut<'a, T> {
    pub fn is_some(&self) -> bool {
        self.slot.is_some()
    }

    pub fn get(&self) -> Option<T::Ref<'_>> {
        self.slot.as_ref().map(|t| T::borrow(t, Loan::conjure()))
    }

    pub fn as_mut(&mut self) -> Option<T::Mut<'_>> {
        self.slot
            .as_mut()
            .map(|t| T::borrow_mut(t, Loan::conjure()))
    }

    pub fn set(&mut self, value: Option<T>) {
        *self.slot = value.map(|t| t.rebrand(erased()));
    }

    pub fn take(&mut self) -> Option<T> {
        self.slot.take().map(|t| T::restore(&t, erased()))
    }
}

impl<T: BorrowMut> BorrowMut for Option<T> {
    type Mut<'a> = OptionMut<'a, T>;
    fn borrow_mut<'a>(from: &'a mut Option<T::Of<'static>>, _: Loan<'a>) -> OptionMut<'a, T> {
        OptionMut { slot: from }
    }
}

/// A stored `Vec`, read: a slice of stored elements, each branded on the way
/// out. It isn't a slice: no `Index`, no `&[T]`, so `list[i]`, slice
/// patterns and a function taking `&[T]` don't apply.
pub struct ListRef<'a, T: Rebrand> {
    items: &'a [T::Of<'static>],
}

impl<'a, T: Rebrand> Clone for ListRef<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T: Rebrand> Copy for ListRef<'a, T> {}

impl<'a, T: Borrow> ListRef<'a, T> {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<T::Ref<'a>> {
        self.items.get(i).map(|t| T::borrow(t, Loan::conjure()))
    }

    pub fn iter(&self) -> impl Iterator<Item = T::Ref<'a>> + 'a {
        self.items.iter().map(|t| T::borrow(t, Loan::conjure()))
    }
}

/// A brand-free element is its own stored copy, so its slice is handed out
/// whole: `Vec<u32>` keeps the slice API that `Vec<Cell>` loses.
impl<'a, T: Rebrand<Of<'static> = T>> ListRef<'a, T> {
    pub fn as_slice(&self) -> &'a [T] {
        self.items
    }
}

impl<T: Borrow> Borrow for Vec<T> {
    type Ref<'a> = ListRef<'a, T>;
    fn borrow<'a>(from: &'a Vec<T::Of<'static>>, _: Loan<'a>) -> ListRef<'a, T> {
        ListRef { items: from }
    }
}

/// A stored `Vec`, written. Every method that takes or gives an element does
/// it at the writer's brand, by view or by a restored copy; every closure
/// sees views. No `&mut Vec<T::Of<'static>>` or `&mut [T]` is handed out,
/// so what the slice API offers beyond these methods is lost.
pub struct ListMut<'a, T: Rebrand> {
    items: &'a mut Vec<T::Of<'static>>,
}

impl<'a, T: BorrowMut> ListMut<'a, T> {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn as_ref(&self) -> ListRef<'_, T> {
        ListRef { items: self.items }
    }

    pub fn get(&self, i: usize) -> Option<T::Ref<'_>> {
        self.items.get(i).map(|t| T::borrow(t, Loan::conjure()))
    }

    pub fn get_mut(&mut self, i: usize) -> Option<T::Mut<'_>> {
        self.items
            .get_mut(i)
            .map(|t| T::borrow_mut(t, Loan::conjure()))
    }

    pub fn iter(&self) -> impl Iterator<Item = T::Ref<'_>> + '_ {
        self.items.iter().map(|t| T::borrow(t, Loan::conjure()))
    }

    /// Disjoint element views, as `iter_mut` gives disjoint `&mut T`.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = T::Mut<'_>> + '_ {
        self.items
            .iter_mut()
            .map(|t| T::borrow_mut(t, Loan::conjure()))
    }

    pub fn set(&mut self, i: usize, value: T) {
        self.items[i] = value.rebrand(erased());
    }

    pub fn push(&mut self, value: T) {
        self.items.push(value.rebrand(erased()));
    }

    pub fn insert(&mut self, i: usize, value: T) {
        self.items.insert(i, value.rebrand(erased()));
    }

    pub fn pop(&mut self) -> Option<T> {
        self.items.pop().map(|t| T::restore(&t, erased()))
    }

    pub fn remove(&mut self, i: usize) -> T {
        T::restore(&self.items.remove(i), erased())
    }

    pub fn swap(&mut self, i: usize, j: usize) {
        self.items.swap(i, j);
    }

    pub fn truncate(&mut self, len: usize) {
        self.items.truncate(len);
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// `f` sees each element at the writer's brand. The stored `Vec` runs
    /// its own `retain`; `f` never gets the `&T::Of<'static>` it iterates.
    pub fn retain(&mut self, mut f: impl FnMut(T::Ref<'_>) -> bool) {
        self.items.retain(|t| f(T::borrow(t, Loan::conjure())));
    }

    pub fn retain_mut(&mut self, mut f: impl FnMut(T::Mut<'_>) -> bool) {
        self.items
            .retain_mut(|t| f(T::borrow_mut(t, Loan::conjure())));
    }

    /// As `retain`: std's stable sort over the stored elements, comparing
    /// views of them.
    pub fn sort_by(&mut self, mut f: impl FnMut(T::Ref<'_>, T::Ref<'_>) -> std::cmp::Ordering) {
        self.items
            .sort_by(|a, b| f(T::borrow(a, Loan::conjure()), T::borrow(b, Loan::conjure())));
    }

    /// A key can't borrow from its element, as with std's `sort_by_key`.
    pub fn sort_by_key<K: Ord>(&mut self, mut f: impl FnMut(T::Ref<'_>) -> K) {
        self.items.sort_by_key(|t| f(T::borrow(t, Loan::conjure())));
    }
}

/// As `ListRef::as_slice`: a brand-free element's stored `Vec` is its own.
impl<'a, T: Rebrand<Of<'static> = T>> ListMut<'a, T> {
    pub fn as_mut_vec(&mut self) -> &mut Vec<T> {
        self.items
    }
}

impl<T: BorrowMut> BorrowMut for Vec<T> {
    type Mut<'a> = ListMut<'a, T>;
    fn borrow_mut<'a>(from: &'a mut Vec<T::Of<'static>>, _: Loan<'a>) -> ListMut<'a, T> {
        ListMut { items: from }
    }
}

// ---------------------------------------------------------------------------
// The engine's side: `sample_ref`, a read-through memo, `update`

/// RFD 4's memo for a read-through cell: a `OnceCell` filled by the first
/// read after a step and replaced, empty, by the step's commit. It holds
/// `B::Of<'static>`, as every stored value does, behind `Rc<dyn Any>`.
struct Memo {
    source: Id,
    compute: Rc<dyn Fn(&dyn Any) -> Rc<dyn Any>>,
    value: OnceCell<Rc<dyn Any>>,
}

impl Memo {
    fn empty(source: Id, compute: &Rc<dyn Fn(&dyn Any) -> Rc<dyn Any>>) -> Rc<dyn Any> {
        Rc::new(Memo {
            source,
            compute: compute.clone(),
            value: OnceCell::new(),
        })
    }
}

impl<'g, A: Rebrand> Cell<'g, A> {
    /// RFD 4's `map_cell`, read-through: `f` runs on the first `sample_ref`
    /// after the input steps, not at all if nothing reads, and its value is
    /// memoized until the next step. The toy reads this cell through
    /// `sample_ref` only; `sample`, `listen_cell` and the adapters expect a
    /// stored value, not a memo.
    pub fn map_cell_lazy<B: Rebrand + Trace, F: Fn(&A) -> B + GraphSafe + 'static>(
        self,
        b: &mut Build<'g>,
        f: F,
    ) -> Cell<'g, B> {
        let source = b.core.live(self.id);
        let compute: Rc<dyn Fn(&dyn Any) -> Rc<dyn Any>> =
            Rc::new(move |x: &dyn Any| store(&with_loaded(x, |a: &A| f(a))));
        let first = Memo::empty(source, &compute);
        // A step of the input clears the memo at commit, by replacing it.
        let next: Next = Box::new(move |view: &View, _: &dyn Any| {
            view.fired(source).map(|_| Memo::empty(source, &compute))
        });
        let trace: TraceFn = Box::new(|value: &dyn Any, found: &mut Vec<Id>| {
            let memo = value
                .downcast_ref::<Memo>()
                .expect("a read-through cell holds a memo");
            if let Some(value) = memo.value.get() {
                found.extend(traced(&load::<B>(&**value)));
            }
        });
        Cell::at(b.core.mint(
            vec![source],
            Kind::Cell {
                value: first,
                next: Some(next),
                trace,
            },
        ))
    }
}

impl<'g> Build<'g> {
    /// RFD 4's `sample`, which returns `&A`, as a view instead: `A::Ref<'_>`
    /// over the stored `A::Of<'static>`, with tokens at this `mutate`'s brand
    /// `'g`. It borrows `self`, as `&A` from the memo does, so it lives no
    /// longer than the sample's borrow, and a `send` while it is held is a
    /// borrow error. A read-through cell's memo is filled on the way.
    pub fn sample_ref<A: Borrow>(&self, cell: Cell<'g, A>) -> Result<A::Ref<'_>, Error> {
        self.core.check(cell.id)?;
        let value = self.core.nodes[cell.id.index as usize].value();
        let stored = match value.downcast_ref::<Memo>() {
            Some(memo) => &**memo.value.get_or_init(|| {
                (memo.compute)(self.core.nodes[memo.source.index as usize].value())
            }),
            None => value,
        };
        let stored = stored
            .downcast_ref::<A::Of<'static>>()
            .expect("a stored value has the type its node was built with");
        Ok(A::borrow(stored, Loan::conjure()))
    }

    /// The write view's vehicle: `f` gets the cell's committed value in
    /// place, as a `Place` at this `mutate`'s brand. RFD 4's `accumulate_mut`
    /// runs its `f` at commit with `&mut S`; the toy's commit swaps immutable
    /// `Rc`s and gives a `next` no `&mut`, so this I/O-side write stands in
    /// for it: the same view, reached through `&mut self` between units. It
    /// runs no unit and fires nothing.
    pub fn update<S: BorrowMut>(
        &mut self,
        cell: Cell<'g, S>,
        f: impl FnOnce(Place<'_, S>),
    ) -> Result<(), Error> {
        self.core.check(cell.id)?;
        let Kind::Cell { value, .. } = &mut self.core.nodes[cell.id.index as usize].kind else {
            panic!("a cell token names a cell");
        };
        let value = Rc::get_mut(value).expect("between units, a committed value has one owner");
        let slot = value
            .downcast_mut::<S::Of<'static>>()
            .expect("a stored value has the type its node was built with");
        f(Place { slot });
        Ok(())
    }
}
