//! RFD 3, performance: what does safe rebranding cost on the operations that
//! read a held value, against gc-arena's free `unsafe` cast?
//!
//! `rfd-0003-brand-erasure` showed a lifetime brand on tokens can be changed
//! with no `unsafe`: a `Rebrand` trait a derive writes field by field, values
//! stored at brand `'static` in a `dyn Any`, and every stored closure behind a
//! wrapper that restores its argument at the brand it was written at. A token
//! restores by copying its id, so the restore is sound, but it is a copy, made
//! on every read of a held value that contains tokens. gc-arena casts the
//! reference instead, which costs nothing, and needs `unsafe`.
//!
//! This module copies that probe's `Rebrand` (same trait, same impls for
//! tokens, `Vec` and `Option`) and reads a held value four ways:
//!
//! - `baseline`: gc-arena's route, a pointer cast from `&T::Of<'static>` to
//!   `&T`. **This is the one `unsafe` block in the probe, an experiment to
//!   price the alternative, not Bough code:** Bough's core forbids `unsafe`.
//!   It is sound only because `T` and `T::Of<'static>` differ in lifetimes
//!   alone, which the derive would guarantee, as gc-arena's `Rootable` does.
//! - `restore`: the earlier probe's `load`, a restored copy on every read.
//! - `view`: its `with_loaded`, which borrows a brand-free value in place and
//!   restores anything else, so it differs from `restore` only on `u64`.
//! - `borrow`: a safe alternative this probe adds, `Borrow`, which a derive
//!   could write next to `Rebrand`: a borrowed view of the stored copy, with
//!   each token rebuilt at the reader's brand by copying its id, data fields
//!   borrowed, and a collection a borrowed slice whose `get` brands one
//!   element. No allocation, but the closure gets `T::Ref<'_>`, not `&T`:
//!   an API change.
//!
//! The shapes: a `u64`, `Three` (three tokens and 16 bytes of data), a
//! `Vec` of 10 and of 1,000 tokens, and `Nested` (a `String`, a `Three`, a
//! `Vec` of four `Three` and an `Option` of a token). Each read hands the
//! value to a function that does O(1) work on it (a length, one element,
//! one field), which is the worst case for the ratio: an O(n) function would
//! hide an O(n) copy.
//!
//! The operations, each one read per event:
//!
//! - `read`: the read alone, `n` times, for the per-read cost.
//! - `snapshot`: a node of a 22-node propagation (an input, four chains of
//!   four maps, three merges, the snapshot, a hold) reads the cell.
//! - `switch_cell`: the same node reads an outer cell holding a token, then
//!   the inner it selects, as a read-through switch does per event.
//! - `sample`: I/O code sends, then samples the cell after commit.
//! - `listener`: a `listen_cell` listener gets the committed value after
//!   commit, through a stored closure.
//!
//! The propagation is a rank-ordered heap over dependents lists, with each
//! node's eval behind a `Box<dyn Fn>`, so the per-event context is roughly
//! what an engine pays for 22 nodes, not a bare loop.
//!
//! A collection can't be restored in place: the read has `&Vec<Cell<'static>>`,
//! so the restore must build a new `Vec`, one allocation and an O(n) copy per
//! read. An owned rebrand (the store side) could reuse the buffer through
//! std's in-place `into_iter().map().collect()`, but reads never own the value.
//! `borrow` is what the safe route has to do instead to avoid the copy.

use std::any::Any;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::hint::black_box;
use std::marker::PhantomData;

/// Invariant in `'g`, so two brands never unify.
type Brand<'g> = PhantomData<fn(&'g ()) -> &'g ()>;

// ---------------------------------------------------------------------------
// The brand and how it changes, as in `rfd-0003-brand-erasure`

/// A proof that brand `'x` may be minted; only this module makes one.
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

/// The earlier probe's trait, unchanged.
pub trait Rebrand: Sized {
    type Of<'x>: 'x;

    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x>;

    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self;

    /// A borrow of the stored copy, for a type with no brand in it.
    fn view<'a>(from: &'a Self::Of<'static>) -> Option<&'a Self> {
        let _ = from;
        None
    }
}

/// The safe way to read without copying: a view of the stored copy that
/// hands out tokens at the reader's brand. A derive writes it next to
/// `Rebrand`, generating a `FooRef<'a, 'g>` per struct.
pub trait Borrow: Rebrand {
    type Ref<'a>: Use
    where
        Self: 'a;

    fn borrow<'a>(from: &'a Self::Of<'static>, at: Witness<'static>) -> Self::Ref<'a>
    where
        Self: 'a;
}

/// What the reading closure does with the value: O(1) work, the same for a
/// value and its `Ref`, so every variant computes the same number.
pub trait Use {
    fn use_it(&self, x: u64) -> u64;
}

impl<T: Use + ?Sized> Use for &T {
    fn use_it(&self, x: u64) -> u64 {
        (**self).use_it(x)
    }
}

impl Rebrand for u64 {
    type Of<'x> = u64;
    fn rebrand<'x>(&self, _: Witness<'x>) -> u64 {
        *self
    }
    fn restore<'x>(from: &u64, _: Witness<'x>) -> u64 {
        *from
    }
    fn view(from: &u64) -> Option<&u64> {
        Some(from)
    }
}

impl Borrow for u64 {
    type Ref<'a> = &'a u64;
    fn borrow<'a>(from: &'a u64, _: Witness<'static>) -> &'a u64
    where
        Self: 'a,
    {
        from
    }
}

impl Use for u64 {
    fn use_it(&self, x: u64) -> u64 {
        self.wrapping_add(x)
    }
}

impl Rebrand for String {
    type Of<'x> = String;
    fn rebrand<'x>(&self, _: Witness<'x>) -> String {
        self.clone()
    }
    fn restore<'x>(from: &String, _: Witness<'x>) -> String {
        from.clone()
    }
    fn view(from: &String) -> Option<&String> {
        Some(from)
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

/// A borrowed slice of stored elements; `get` brands one on the way out.
pub struct ListRef<'a, T: Rebrand + 'a> {
    items: &'a [T::Of<'static>],
    at: Witness<'static>,
}

impl<'a, T: Borrow + 'a> ListRef<'a, T> {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, i: usize) -> T::Ref<'a> {
        T::borrow(&self.items[i], self.at)
    }
}

impl<T: Borrow> Borrow for Vec<T> {
    type Ref<'a>
        = ListRef<'a, T>
    where
        T: 'a;
    fn borrow<'a>(from: &'a Vec<T::Of<'static>>, at: Witness<'static>) -> ListRef<'a, T>
    where
        T: 'a,
    {
        ListRef { items: from, at }
    }
}

/// The length and one element, chosen by `x`.
impl<T: Use> Use for Vec<T> {
    fn use_it(&self, x: u64) -> u64 {
        let i = x as usize % self.len();
        (self.len() as u64).wrapping_add(self[i].use_it(x))
    }
}

impl<'a, T: Borrow + 'a> Use for ListRef<'a, T> {
    fn use_it(&self, x: u64) -> u64 {
        let i = x as usize % self.len();
        (self.len() as u64).wrapping_add(self.get(i).use_it(x))
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

impl<T: Borrow> Borrow for Option<T> {
    type Ref<'a>
        = Option<T::Ref<'a>>
    where
        T: 'a;
    fn borrow<'a>(from: &'a Option<T::Of<'static>>, at: Witness<'static>) -> Self::Ref<'a>
    where
        T: 'a,
    {
        from.as_ref().map(|t| T::borrow(t, at))
    }
}

impl<T: Use> Use for Option<T> {
    fn use_it(&self, x: u64) -> u64 {
        self.as_ref().map_or(0, |t| t.use_it(x))
    }
}

// ---------------------------------------------------------------------------
// Tokens

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Id {
    index: u32,
    generation: u32,
}

/// A cell token: an id and a brand, `Copy`.
pub struct Cell<'g, A> {
    id: Id,
    event: PhantomData<fn() -> A>,
    brand: Brand<'g>,
}

impl<'g, A> Clone for Cell<'g, A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'g, A> Copy for Cell<'g, A> {}

impl<'g, A> Cell<'g, A> {
    fn at(id: Id) -> Self {
        Cell {
            id,
            event: PhantomData,
            brand: PhantomData,
        }
    }

    fn new(index: u32) -> Self {
        Cell::at(Id {
            index,
            generation: 1,
        })
    }
}

/// A token rebrands by copying its id: the whole of the safe route.
impl<'g, A: Rebrand> Rebrand for Cell<'g, A> {
    type Of<'x> = Cell<'x, A::Of<'x>>;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Self::Of<'x> {
        Cell::at(self.id)
    }
    fn restore<'x>(from: &Self::Of<'x>, _: Witness<'x>) -> Self {
        Cell::at(from.id)
    }
}

impl<'g, A: Rebrand> Borrow for Cell<'g, A> {
    type Ref<'a>
        = Cell<'g, A>
    where
        Self: 'a;
    fn borrow<'a>(from: &'a Cell<'static, A::Of<'static>>, _: Witness<'static>) -> Self
    where
        Self: 'a,
    {
        Cell::at(from.id)
    }
}

impl<'g, A> Use for Cell<'g, A> {
    fn use_it(&self, x: u64) -> u64 {
        u64::from(self.id.index).wrapping_add(x)
    }
}

// ---------------------------------------------------------------------------
// The shapes, with the impls a derive would write

/// Three tokens and some data, 40 bytes.
pub struct Three<'g> {
    pub a: Cell<'g, u64>,
    pub b: Cell<'g, u64>,
    pub c: Cell<'g, u64>,
    pub weight: u64,
    pub count: u32,
    pub flags: u32,
}

impl<'g> Rebrand for Three<'g> {
    type Of<'x> = Three<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Three<'x> {
        Three {
            a: self.a.rebrand(to),
            b: self.b.rebrand(to),
            c: self.c.rebrand(to),
            weight: self.weight,
            count: self.count,
            flags: self.flags,
        }
    }
    fn restore<'x>(from: &Three<'x>, at: Witness<'x>) -> Self {
        Three {
            a: Rebrand::restore(&from.a, at),
            b: Rebrand::restore(&from.b, at),
            c: Rebrand::restore(&from.c, at),
            weight: from.weight,
            count: from.count,
            flags: from.flags,
        }
    }
}

/// What a derived `Borrow` would generate: tokens at the reader's brand,
/// data borrowed.
pub struct ThreeRef<'a, 'g> {
    pub a: Cell<'g, u64>,
    pub b: Cell<'g, u64>,
    pub c: Cell<'g, u64>,
    pub weight: &'a u64,
    pub count: &'a u32,
    pub flags: &'a u32,
}

impl<'g> Borrow for Three<'g> {
    type Ref<'a>
        = ThreeRef<'a, 'g>
    where
        Self: 'a;
    fn borrow<'a>(from: &'a Three<'static>, at: Witness<'static>) -> ThreeRef<'a, 'g>
    where
        Self: 'a,
    {
        ThreeRef {
            a: Cell::<u64>::borrow(&from.a, at),
            b: Cell::<u64>::borrow(&from.b, at),
            c: Cell::<u64>::borrow(&from.c, at),
            weight: &from.weight,
            count: &from.count,
            flags: &from.flags,
        }
    }
}

impl<'g> Use for Three<'g> {
    fn use_it(&self, x: u64) -> u64 {
        self.a.use_it(x).wrapping_add(self.weight)
    }
}

impl<'a, 'g> Use for ThreeRef<'a, 'g> {
    fn use_it(&self, x: u64) -> u64 {
        self.a.use_it(x).wrapping_add(*self.weight)
    }
}

/// A screen's state: a name, a header, a few rows, a selection.
pub struct Nested<'g> {
    pub name: String,
    pub header: Three<'g>,
    pub rows: Vec<Three<'g>>,
    pub selected: Option<Cell<'g, u64>>,
}

impl<'g> Rebrand for Nested<'g> {
    type Of<'x> = Nested<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Nested<'x> {
        Nested {
            name: self.name.rebrand(to),
            header: self.header.rebrand(to),
            rows: self.rows.rebrand(to),
            selected: self.selected.rebrand(to),
        }
    }
    fn restore<'x>(from: &Nested<'x>, at: Witness<'x>) -> Self {
        Nested {
            name: Rebrand::restore(&from.name, at),
            header: Rebrand::restore(&from.header, at),
            rows: Rebrand::restore(&from.rows, at),
            selected: Rebrand::restore(&from.selected, at),
        }
    }
}

pub struct NestedRef<'a, 'g> {
    pub name: &'a str,
    pub header: ThreeRef<'a, 'g>,
    pub rows: ListRef<'a, Three<'g>>,
    pub selected: Option<Cell<'g, u64>>,
}

impl<'g> Borrow for Nested<'g> {
    type Ref<'a>
        = NestedRef<'a, 'g>
    where
        Self: 'a;
    fn borrow<'a>(from: &'a Nested<'static>, at: Witness<'static>) -> NestedRef<'a, 'g>
    where
        Self: 'a,
    {
        NestedRef {
            name: &from.name,
            header: Three::borrow(&from.header, at),
            rows: Vec::<Three<'g>>::borrow(&from.rows, at),
            selected: Option::<Cell<'g, u64>>::borrow(&from.selected, at),
        }
    }
}

impl<'g> Use for Nested<'g> {
    fn use_it(&self, x: u64) -> u64 {
        (self.name.len() as u64)
            .wrapping_add(self.header.use_it(x))
            .wrapping_add(self.rows.use_it(x))
            .wrapping_add(self.selected.use_it(x))
    }
}

impl<'a, 'g> Use for NestedRef<'a, 'g> {
    fn use_it(&self, x: u64) -> u64 {
        (self.name.len() as u64)
            .wrapping_add(self.header.use_it(x))
            .wrapping_add(self.rows.use_it(x))
            .wrapping_add(self.selected.use_it(x))
    }
}

fn three<'g>(k: u32) -> Three<'g> {
    Three {
        a: Cell::new(3 * k),
        b: Cell::new(3 * k + 1),
        c: Cell::new(3 * k + 2),
        weight: u64::from(k) * 10,
        count: k,
        flags: 0b101,
    }
}

/// A family of shapes, one type per brand, so the engine can name the shape
/// at the brand a closure was written at.
pub trait Family: 'static {
    type At<'g>: Borrow + Use;
    fn make<'g>() -> Self::At<'g>;
}

pub struct U64s;
pub struct Threes;
pub struct Vec10;
pub struct Vec1000;
pub struct Nesteds;

impl Family for U64s {
    type At<'g> = u64;
    fn make<'g>() -> Self::At<'g> {
        42
    }
}

impl Family for Threes {
    type At<'g> = Three<'g>;
    fn make<'g>() -> Three<'g> {
        three(1)
    }
}

impl Family for Vec10 {
    type At<'g> = Vec<Cell<'g, u64>>;
    fn make<'g>() -> Self::At<'g> {
        (0..10).map(Cell::new).collect()
    }
}

impl Family for Vec1000 {
    type At<'g> = Vec<Cell<'g, u64>>;
    fn make<'g>() -> Self::At<'g> {
        (0..1000).map(Cell::new).collect()
    }
}

impl Family for Nesteds {
    type At<'g> = Nested<'g>;
    fn make<'g>() -> Nested<'g> {
        Nested {
            name: String::from("inventory panel"),
            header: three(0),
            rows: (1..=4).map(three).collect(),
            selected: Some(Cell::new(7)),
        }
    }
}

// ---------------------------------------------------------------------------
// The four reads

fn typed<T: Rebrand>(stored: &dyn Any) -> &T::Of<'static> {
    stored
        .downcast_ref::<T::Of<'static>>()
        .expect("a stored value has the type its node was built with")
}

/// One way to read a stored value at the reader's brand and run the
/// reader's function on it. `black_box` on what the function gets keeps the
/// optimizer from reading one field of the stored copy and skipping the copy,
/// which it can't do once the function is the user's, behind a `dyn`.
pub trait Reader: 'static {
    fn read<T: Borrow + Use>(stored: &dyn Any, x: u64) -> u64;
}

/// gc-arena's route: a cast.
pub struct Transmute;
/// The earlier probe's `load`.
pub struct Restore;
/// The earlier probe's `with_loaded`.
pub struct View;
/// A derived borrowed view.
pub struct Borrowed;

impl Reader for Transmute {
    fn read<T: Borrow + Use>(stored: &dyn Any, x: u64) -> u64 {
        let stored: *const T::Of<'static> = typed::<T>(stored);
        // SAFETY (experiment only; Bough forbids `unsafe`): `T::Of<'static>`
        // and `T` are one type up to lifetimes, so they share a layout, and
        // the returned borrow lives no longer than `stored`'s.
        let value: &T = unsafe { &*stored.cast::<T>() };
        black_box(value).use_it(x)
    }
}

impl Reader for Restore {
    fn read<T: Borrow + Use>(stored: &dyn Any, x: u64) -> u64 {
        let value = T::restore(typed::<T>(stored), erased());
        black_box(&value).use_it(x)
    }
}

impl Reader for View {
    fn read<T: Borrow + Use>(stored: &dyn Any, x: u64) -> u64 {
        let stored = typed::<T>(stored);
        match T::view(stored) {
            Some(value) => black_box(value).use_it(x),
            None => {
                let value = T::restore(stored, erased());
                black_box(&value).use_it(x)
            }
        }
    }
}

impl Reader for Borrowed {
    fn read<T: Borrow + Use>(stored: &dyn Any, x: u64) -> u64 {
        let value = T::borrow(typed::<T>(stored), erased());
        black_box(&value).use_it(x)
    }
}

/// A read written at some brand `'g`, as a stored closure is: the brand is
/// whatever the compiler picks, and lifetimes are erased before codegen.
fn read_at<R: Reader, F: Family>(stored: &dyn Any, x: u64) -> u64 {
    fn at<'g, R: Reader, F: Family>(stored: &dyn Any, x: u64, _: Witness<'g>) -> u64 {
        R::read::<F::At<'g>>(stored, x)
    }
    at::<R, F>(stored, x, Witness::conjure())
}

/// The switch's read: the outer cell's token, then the inner it selects.
fn read_switch<R: Reader, F: Family>(cells: &[Box<dyn Any>], x: u64) -> u64 {
    fn at<'g, R: Reader, F: Family>(cells: &[Box<dyn Any>], x: u64, _: Witness<'g>) -> u64 {
        let inner = R::read::<Cell<'g, F::At<'g>>>(&*cells[OUTER], 0) as usize;
        R::read::<F::At<'g>>(&*cells[inner], x)
    }
    at::<R, F>(cells, x, Witness::conjure())
}

// ---------------------------------------------------------------------------
// The propagation

/// The held cell every operation reads, and the switch's outer cell.
const HELD: usize = 0;
const OUTER: usize = 1;

type Eval = Box<dyn Fn(&[Box<dyn Any>], u64) -> u64>;

struct Node {
    inputs: Vec<usize>,
    dependents: Vec<usize>,
    eval: Eval,
}

/// 22 nodes in rank order: an input, four chains of four maps, three
/// merges, the node under test (or a map), a hold.
struct Net {
    nodes: Vec<Node>,
    fired: Vec<Option<u64>>,
    queued: Vec<bool>,
    heap: BinaryHeap<Reverse<usize>>,
    touched: Vec<usize>,
    cells: Vec<Box<dyn Any>>,
    held: u64,
}

impl Net {
    fn new(cells: Vec<Box<dyn Any>>, probe: Option<Eval>) -> Self {
        let mut specs: Vec<(Vec<usize>, Eval)> = vec![(vec![], Box::new(|_, e| e))];
        for chain in 0..4u64 {
            for k in 0..4u64 {
                let from = if k == 0 { 0 } else { specs.len() - 1 };
                let salt = chain * 4 + k;
                specs.push((
                    vec![from],
                    Box::new(move |_, e: u64| e.wrapping_mul(31).wrapping_add(salt)),
                ));
            }
        }
        let merge = || -> Eval { Box::new(|_, e| e) };
        specs.push((vec![4, 8], merge()));
        specs.push((vec![12, 16], merge()));
        specs.push((vec![17, 18], merge()));
        let probe = probe.unwrap_or_else(|| Box::new(|_, e: u64| e ^ 0x5a));
        specs.push((vec![19], probe));
        specs.push((vec![20], Box::new(|_, e| e)));

        let mut nodes: Vec<Node> = specs
            .into_iter()
            .map(|(inputs, eval)| Node {
                inputs,
                dependents: Vec::new(),
                eval,
            })
            .collect();
        for i in 0..nodes.len() {
            for j in nodes[i].inputs.clone() {
                nodes[j].dependents.push(i);
            }
        }
        let n = nodes.len();
        Net {
            nodes,
            fired: vec![None; n],
            queued: vec![false; n],
            heap: BinaryHeap::new(),
            touched: Vec::with_capacity(n),
            cells,
            held: 0,
        }
    }

    /// One transaction: fire the input, evaluate in rank order, commit.
    fn send(&mut self, x: u64) {
        self.fired[0] = Some(x);
        self.touched.push(0);
        self.enqueue(0);
        while let Some(Reverse(i)) = self.heap.pop() {
            let node = &self.nodes[i];
            // A merge combines what fired; every other node has one input.
            let event = node
                .inputs
                .iter()
                .filter_map(|&j| self.fired[j])
                .fold(0u64, |a, e| a.wrapping_add(e));
            let out = (node.eval)(&self.cells, event);
            self.fired[i] = Some(out);
            self.touched.push(i);
            self.enqueue(i);
        }
        // Commit: the hold takes the last node's event, streams clear.
        let last = self.nodes.len() - 1;
        if let Some(v) = self.fired[last] {
            self.held = v;
        }
        for i in self.touched.drain(..) {
            self.fired[i] = None;
            self.queued[i] = false;
        }
    }

    fn enqueue(&mut self, i: usize) {
        for &d in &self.nodes[i].dependents {
            if !self.queued[d] {
                self.queued[d] = true;
                self.heap.push(Reverse(d));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The harness the benches drive

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    Baseline,
    Restore,
    View,
    Borrow,
}

impl Variant {
    pub const ALL: [Variant; 4] = [
        Variant::Baseline,
        Variant::Restore,
        Variant::View,
        Variant::Borrow,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Variant::Baseline => "baseline",
            Variant::Restore => "restore",
            Variant::View => "view",
            Variant::Borrow => "borrow",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    U64,
    Three,
    Vec10,
    Vec1000,
    Nested,
}

impl Shape {
    pub const ALL: [Shape; 5] = [
        Shape::U64,
        Shape::Three,
        Shape::Vec10,
        Shape::Vec1000,
        Shape::Nested,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Shape::U64 => "u64",
            Shape::Three => "three",
            Shape::Vec10 => "vec10",
            Shape::Vec1000 => "vec1000",
            Shape::Nested => "nested",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Read,
    Snapshot,
    SwitchCell,
    Sample,
    Listener,
}

impl Op {
    pub const ALL: [Op; 5] = [
        Op::Read,
        Op::Snapshot,
        Op::SwitchCell,
        Op::Sample,
        Op::Listener,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Op::Read => "read",
            Op::Snapshot => "snapshot",
            Op::SwitchCell => "switch_cell",
            Op::Sample => "sample",
            Op::Listener => "listener",
        }
    }
}

trait Run {
    fn run(&mut self, n: u64) -> u64;
}

struct Harness<R, F> {
    op: Op,
    net: Net,
    listener: Eval,
    out: u64,
    kinds: PhantomData<fn() -> (R, F)>,
}

impl<R: Reader, F: Family> Harness<R, F> {
    fn new(op: Op) -> Self {
        let cells = cells::<F>();
        let probe: Option<Eval> = match op {
            Op::Snapshot => Some(Box::new(|cells, e| read_at::<R, F>(&*cells[HELD], e))),
            Op::SwitchCell => Some(Box::new(read_switch::<R, F>)),
            _ => None,
        };
        Harness {
            op,
            net: Net::new(cells, probe),
            listener: Box::new(|cells, e| read_at::<R, F>(&*cells[HELD], e)),
            out: 0,
            kinds: PhantomData,
        }
    }
}

/// The held value and an outer cell selecting it, stored at `'static`.
fn cells<F: Family>() -> Vec<Box<dyn Any>> {
    fn at<'g, F: Family>(_: Witness<'g>) -> Vec<Box<dyn Any>> {
        let held: F::At<'g> = F::make();
        let outer: Cell<'g, F::At<'g>> = Cell::new(HELD as u32);
        vec![
            Box::new(held.rebrand(erased())),
            Box::new(outer.rebrand(erased())),
        ]
    }
    at::<F>(Witness::conjure())
}

impl<R: Reader, F: Family> Run for Harness<R, F> {
    fn run(&mut self, n: u64) -> u64 {
        for x in 0..n {
            match self.op {
                Op::Read => {
                    let stored = black_box(&*self.net.cells[HELD]);
                    self.out = self.out.wrapping_add(read_at::<R, F>(stored, x));
                }
                Op::Snapshot | Op::SwitchCell => self.net.send(x),
                Op::Sample => {
                    self.net.send(x);
                    let v = read_at::<R, F>(&*self.net.cells[HELD], x);
                    self.out = self.out.wrapping_add(v);
                }
                Op::Listener => {
                    self.net.send(x);
                    let v = (self.listener)(&self.net.cells, x);
                    self.out = self.out.wrapping_add(v);
                }
            }
        }
        self.out.wrapping_add(self.net.held)
    }
}

/// One (operation, variant, shape), built outside what is measured.
pub struct Probe {
    inner: Box<dyn Run>,
}

impl Probe {
    pub fn new(op: Op, variant: Variant, shape: Shape) -> Self {
        fn by_shape<R: Reader>(op: Op, shape: Shape) -> Box<dyn Run> {
            match shape {
                Shape::U64 => Box::new(Harness::<R, U64s>::new(op)),
                Shape::Three => Box::new(Harness::<R, Threes>::new(op)),
                Shape::Vec10 => Box::new(Harness::<R, Vec10>::new(op)),
                Shape::Vec1000 => Box::new(Harness::<R, Vec1000>::new(op)),
                Shape::Nested => Box::new(Harness::<R, Nesteds>::new(op)),
            }
        }
        let inner = match variant {
            Variant::Baseline => by_shape::<Transmute>(op, shape),
            Variant::Restore => by_shape::<Restore>(op, shape),
            Variant::View => by_shape::<View>(op, shape),
            Variant::Borrow => by_shape::<Borrowed>(op, shape),
        };
        Probe { inner }
    }

    /// `n` reads for `Op::Read`, `n` events for the rest. Returns a checksum
    /// that is the same for every variant.
    pub fn run(&mut self, n: u64) -> u64 {
        self.inner.run(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant reads the same value, so the counts compare like work.
    #[test]
    fn variants_agree() {
        for op in Op::ALL {
            for shape in Shape::ALL {
                let sums: Vec<u64> = Variant::ALL
                    .iter()
                    .map(|&v| Probe::new(op, v, shape).run(50))
                    .collect();
                assert!(sums.windows(2).all(|w| w[0] == w[1]), "{op:?} {shape:?}");
            }
        }
    }
}
