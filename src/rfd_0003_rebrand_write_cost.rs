//! RFD 3, performance: what does safe rebranding cost on the operations that
//! *write* a held value, against gc-arena's free `unsafe` cast?
//!
//! `rfd-0003-rebrand-cost` priced the read side: values containing tokens are
//! stored as `T::Of<'static>` in a `dyn Any`, and each read restores a copy at
//! the reader's brand. This probe prices the other half. Storing rebrands a
//! value to `'static` (`rfd-0003-brand-erasure`'s `store`, which takes `&T`
//! and so builds a new copy), and `accumulate_mut`, whose `f: FnMut(A, &mut S)`
//! runs at commit (RFD 4, spike F9/F30), does a whole round trip per event in
//! that fixture: restore a copy of the state, mutate it, rebrand it back.
//!
//! The read probe keeps `Witness`'s constructor, `erased`, the token
//! constructor and its propagation private, so this module copies that
//! machinery rather than importing it: the same `Rebrand` trait, extended
//! with the owned and mutable methods below, the same shapes and the same
//! 22-node propagation, with the node under test able to write.
//!
//! The ways to write, the `Writer`s:
//!
//! - `baseline`: gc-arena's route, a cast: an owned `T` is moved into the
//!   slot as a `T::Of<'static>`, and `accumulate_mut` casts `&mut
//!   T::Of<'static>` to `&mut T` and mutates in place. **The only `unsafe` in
//!   the probe, an experiment pricing the alternative, not Bough code.**
//! - `rebrand`: the brand-erasure fixture's route. A store is
//!   `value.rebrand(erased())` from `&T`, a new copy, and the original is
//!   dropped; an update restores a copy, mutates it and rebrands it back.
//! - `view`: a brand-free type (`u64` here) moves in and is mutated in place
//!   through `view_mut`, safely, because its `Of<'static>` is itself;
//!   anything else goes the `rebrand` route. The write-side twin of the read
//!   probe's `view`.
//! - `owned`: a safe route this probe adds. `into_rebrand(self)` and
//!   `from_restore(Of<'x>)` consume the value, so a derive can move every
//!   field, and a `Vec` rebrands through `into_iter().map().collect()`, which
//!   std runs in place when the element layouts match, as they do here. An
//!   update takes the state out of its slot, restores it by value, mutates it
//!   and puts it back.
//! - `borrow`: for `accumulate_mut` only, a safe mutable view, `BorrowMut`,
//!   that a derive could write next to `Rebrand`: `T::Mut<'_>` over the stored
//!   copy, with each token behind a `TokenMut` whose `get` returns it at the
//!   writer's brand and whose `set` stores a new one's id, data fields as
//!   `&mut`, and a collection as a `ListMut` whose `get_mut` views one
//!   element. No copy and no `unsafe`, but `f` gets `S::Mut<'_>`, not
//!   `&mut S`: the API changes, and a token-bearing `Vec` state loses the
//!   slice API (`sort`, `retain`, `iter_mut`) to whatever `ListMut` offers.
//!
//! The operations:
//!
//! - `store`: a value is made (from the event number) and stored, `n` times,
//!   the per-write cost with the make included in every variant.
//! - `update`: `accumulate_mut`'s commit step alone, `n` times.
//! - `hold`: node 20 of the propagation makes a value from its event and the
//!   engine stores it as the fired event, which the hold moves into its cell
//!   at commit. The fixture stores at emission, not at commit, because a fired
//!   event already lives in a `dyn Any`; either way it is one store per event.
//! - `accumulate`: node 20 is an `accumulate_mut`'s input; at commit the
//!   engine runs the update on the state.
//! - `switch_cell`: node 20 computes a new selection each event (alternating
//!   between two inner cells) and stores the token, which the outer hold
//!   takes at commit; node 21 is the switch, reading the outer cell and the
//!   inner it selects through the cast, so reads cost the same in every
//!   variant and the difference is the write.
//!
//! The shapes and the propagation are the read probe's: `u64`, `Three`
//! (three tokens and 16 bytes of data), a `Vec` of 10 and of 1,000 tokens,
//! and `Nested` (a `String`, a `Three`, a `Vec` of four `Three`, an `Option`
//! of a token). A mutation is O(1): one field, one element, one token, the
//! worst case for the ratio, as on the read side.

use std::any::Any;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::hint::black_box;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;

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

/// The earlier probes' trait, with the owned and in-place methods the write
/// side can use. A derive writes every method field by field.
pub trait Rebrand: Sized {
    type Of<'x>: 'x;

    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x>;

    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self;

    /// `rebrand` by value: fields move, a `Vec` keeps its buffer.
    fn into_rebrand<'x>(self, to: Witness<'x>) -> Self::Of<'x>;

    /// `restore` by value.
    fn from_restore<'x>(from: Self::Of<'x>, at: Witness<'x>) -> Self;

    /// The store-side `view`: a type with no brand is its own `Of<'static>`,
    /// so it moves in. Everything else hands the value back.
    fn into_static(self) -> Result<Self::Of<'static>, Self> {
        Err(self)
    }

    /// The stored copy of a type with no brand, mutably, in place.
    fn view_mut<'a>(from: &'a mut Self::Of<'static>) -> Option<&'a mut Self> {
        let _ = from;
        None
    }
}

/// The safe in-place route for `accumulate_mut`: a mutable view of the
/// stored copy that takes and gives tokens at the writer's brand.
pub trait BorrowMut: Rebrand {
    type Mut<'a>: Poke
    where
        Self: 'a;

    fn borrow_mut<'a>(from: &'a mut Self::Of<'static>) -> Self::Mut<'a>
    where
        Self: 'a;
}

/// What `accumulate_mut`'s `f` does to the state: O(1), the same on a value
/// and on its `Mut`, so every variant ends in the same state.
pub trait Poke {
    fn poke(&mut self, x: u64);
}

/// What reads a value for the checksum: the read probe's `Use`.
pub trait Use {
    fn use_it(&self, x: u64) -> u64;
}

impl Rebrand for u64 {
    type Of<'x> = u64;
    fn rebrand<'x>(&self, _: Witness<'x>) -> u64 {
        *self
    }
    fn restore<'x>(from: &u64, _: Witness<'x>) -> u64 {
        *from
    }
    fn into_rebrand<'x>(self, _: Witness<'x>) -> u64 {
        self
    }
    fn from_restore<'x>(from: u64, _: Witness<'x>) -> u64 {
        from
    }
    fn into_static(self) -> Result<u64, u64> {
        Ok(self)
    }
    fn view_mut(from: &mut u64) -> Option<&mut u64> {
        Some(from)
    }
}

impl BorrowMut for u64 {
    type Mut<'a> = &'a mut u64;
    fn borrow_mut<'a>(from: &'a mut u64) -> &'a mut u64
    where
        Self: 'a,
    {
        from
    }
}

impl Poke for u64 {
    fn poke(&mut self, x: u64) {
        *self = self.wrapping_add(x);
    }
}

impl Poke for &mut u64 {
    fn poke(&mut self, x: u64) {
        (**self).poke(x);
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
    fn into_rebrand<'x>(self, _: Witness<'x>) -> String {
        self
    }
    fn from_restore<'x>(from: String, _: Witness<'x>) -> String {
        from
    }
    fn into_static(self) -> Result<String, String> {
        Ok(self)
    }
    fn view_mut(from: &mut String) -> Option<&mut String> {
        Some(from)
    }
}

impl BorrowMut for String {
    type Mut<'a> = &'a mut String;
    fn borrow_mut<'a>(from: &'a mut String) -> &'a mut String
    where
        Self: 'a,
    {
        from
    }
}

/// The workload never changes a name.
impl Poke for String {
    fn poke(&mut self, _: u64) {}
}

impl Poke for &mut String {
    fn poke(&mut self, _: u64) {}
}

impl<T: Rebrand> Rebrand for Vec<T> {
    type Of<'x> = Vec<T::Of<'x>>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        self.iter().map(|t| t.rebrand(to)).collect()
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        from.iter().map(|t| T::restore(t, at)).collect()
    }
    // In place: std reuses the buffer when `T` and `T::Of<'x>` have one
    // layout, which a type differing only in its brand always does.
    fn into_rebrand<'x>(self, to: Witness<'x>) -> Self::Of<'x> {
        self.into_iter().map(|t| t.into_rebrand(to)).collect()
    }
    fn from_restore<'x>(from: Self::Of<'x>, at: Witness<'x>) -> Self {
        from.into_iter().map(|t| T::from_restore(t, at)).collect()
    }
}

/// The stored elements, mutably; each comes out as its own `Mut`, and a new
/// one goes in rebranded, so no `&mut Vec<T>` at the writer's brand exists.
pub struct ListMut<'a, T: Rebrand + 'a> {
    items: &'a mut Vec<T::Of<'static>>,
}

impl<'a, T: BorrowMut + 'a> ListMut<'a, T> {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get_mut(&mut self, i: usize) -> T::Mut<'_> {
        T::borrow_mut(&mut self.items[i])
    }

    pub fn set(&mut self, i: usize, value: T) {
        self.items[i] = value.into_rebrand(erased());
    }

    /// What a growing accumulator does, in O(1) amortized, as `&mut Vec`'s
    /// push would.
    pub fn push(&mut self, value: T) {
        self.items.push(value.into_rebrand(erased()));
    }
}

impl<T: BorrowMut> BorrowMut for Vec<T> {
    type Mut<'a>
        = ListMut<'a, T>
    where
        T: 'a;
    fn borrow_mut<'a>(from: &'a mut Vec<T::Of<'static>>) -> ListMut<'a, T>
    where
        T: 'a,
    {
        ListMut { items: from }
    }
}

/// One element, chosen by `x`.
impl<T: Poke> Poke for Vec<T> {
    fn poke(&mut self, x: u64) {
        let i = x as usize % self.len();
        self[i].poke(x);
    }
}

impl<'a, T: BorrowMut + 'a> Poke for ListMut<'a, T> {
    fn poke(&mut self, x: u64) {
        let i = x as usize % self.len();
        self.get_mut(i).poke(x);
    }
}

/// The length and one element, chosen by `x`.
impl<T: Use> Use for Vec<T> {
    fn use_it(&self, x: u64) -> u64 {
        let i = x as usize % self.len();
        (self.len() as u64).wrapping_add(self[i].use_it(x))
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
    fn into_rebrand<'x>(self, to: Witness<'x>) -> Self::Of<'x> {
        self.map(|t| t.into_rebrand(to))
    }
    fn from_restore<'x>(from: Self::Of<'x>, at: Witness<'x>) -> Self {
        from.map(|t| T::from_restore(t, at))
    }
}

pub struct OptionMut<'a, T: Rebrand + 'a> {
    slot: &'a mut Option<T::Of<'static>>,
}

impl<'a, T: BorrowMut + 'a> OptionMut<'a, T> {
    pub fn as_mut(&mut self) -> Option<T::Mut<'_>> {
        self.slot.as_mut().map(|t| T::borrow_mut(t))
    }

    pub fn set(&mut self, value: Option<T>) {
        *self.slot = value.map(|t| t.into_rebrand(erased()));
    }
}

impl<T: BorrowMut> BorrowMut for Option<T> {
    type Mut<'a>
        = OptionMut<'a, T>
    where
        T: 'a;
    fn borrow_mut<'a>(from: &'a mut Option<T::Of<'static>>) -> OptionMut<'a, T>
    where
        T: 'a,
    {
        OptionMut { slot: from }
    }
}

impl<T: Poke> Poke for Option<T> {
    fn poke(&mut self, x: u64) {
        if let Some(t) = self {
            t.poke(x);
        }
    }
}

impl<'a, T: BorrowMut + 'a> Poke for OptionMut<'a, T> {
    fn poke(&mut self, x: u64) {
        if let Some(mut t) = self.as_mut() {
            t.poke(x);
        }
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

    /// A token from an event, as `f` would copy one out of its argument.
    fn new(index: u64) -> Self {
        Cell::at(Id {
            index: index as u32,
            generation: 1,
        })
    }
}

/// A token rebrands by copying its id, by reference or by value alike.
impl<'g, A: Rebrand> Rebrand for Cell<'g, A> {
    type Of<'x> = Cell<'x, A::Of<'x>>;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Self::Of<'x> {
        Cell::at(self.id)
    }
    fn restore<'x>(from: &Self::Of<'x>, _: Witness<'x>) -> Self {
        Cell::at(from.id)
    }
    fn into_rebrand<'x>(self, _: Witness<'x>) -> Self::Of<'x> {
        Cell::at(self.id)
    }
    fn from_restore<'x>(from: Self::Of<'x>, _: Witness<'x>) -> Self {
        Cell::at(from.id)
    }
}

/// A stored token, mutably: read and written at the writer's brand `'g`,
/// kept at `'static`. Only an id crosses, so nothing branded leaks.
pub struct TokenMut<'a, 'g, A: Rebrand + 'a> {
    slot: &'a mut Cell<'static, A::Of<'static>>,
    brand: Brand<'g>,
}

impl<'a, 'g, A: Rebrand + 'a> TokenMut<'a, 'g, A> {
    pub fn get(&self) -> Cell<'g, A> {
        Cell::at(self.slot.id)
    }

    pub fn set(&mut self, token: Cell<'g, A>) {
        *self.slot = Cell::at(token.id);
    }
}

impl<'g, A: Rebrand> BorrowMut for Cell<'g, A> {
    type Mut<'a>
        = TokenMut<'a, 'g, A>
    where
        Self: 'a;
    fn borrow_mut<'a>(from: &'a mut Cell<'static, A::Of<'static>>) -> TokenMut<'a, 'g, A>
    where
        Self: 'a,
    {
        TokenMut {
            slot: from,
            brand: PhantomData,
        }
    }
}

/// A new token from the event.
impl<'g, A> Poke for Cell<'g, A> {
    fn poke(&mut self, x: u64) {
        *self = Cell::new(x);
    }
}

impl<'a, 'g, A: Rebrand + 'a> Poke for TokenMut<'a, 'g, A> {
    fn poke(&mut self, x: u64) {
        self.set(Cell::new(x));
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
    fn into_rebrand<'x>(self, to: Witness<'x>) -> Three<'x> {
        Three {
            a: self.a.into_rebrand(to),
            b: self.b.into_rebrand(to),
            c: self.c.into_rebrand(to),
            weight: self.weight,
            count: self.count,
            flags: self.flags,
        }
    }
    fn from_restore<'x>(from: Three<'x>, at: Witness<'x>) -> Self {
        Three {
            a: Rebrand::from_restore(from.a, at),
            b: Rebrand::from_restore(from.b, at),
            c: Rebrand::from_restore(from.c, at),
            weight: from.weight,
            count: from.count,
            flags: from.flags,
        }
    }
}

/// What a derived `BorrowMut` would generate.
pub struct ThreeMut<'a, 'g> {
    pub a: TokenMut<'a, 'g, u64>,
    pub b: TokenMut<'a, 'g, u64>,
    pub c: TokenMut<'a, 'g, u64>,
    pub weight: &'a mut u64,
    pub count: &'a mut u32,
    pub flags: &'a mut u32,
}

impl<'g> BorrowMut for Three<'g> {
    type Mut<'a>
        = ThreeMut<'a, 'g>
    where
        Self: 'a;
    fn borrow_mut<'a>(from: &'a mut Three<'static>) -> ThreeMut<'a, 'g>
    where
        Self: 'a,
    {
        ThreeMut {
            a: Cell::<'g, u64>::borrow_mut(&mut from.a),
            b: Cell::<'g, u64>::borrow_mut(&mut from.b),
            c: Cell::<'g, u64>::borrow_mut(&mut from.c),
            weight: &mut from.weight,
            count: &mut from.count,
            flags: &mut from.flags,
        }
    }
}

impl<'g> Poke for Three<'g> {
    fn poke(&mut self, x: u64) {
        self.a.poke(x);
        self.weight = self.weight.wrapping_add(x);
    }
}

impl<'a, 'g> Poke for ThreeMut<'a, 'g> {
    fn poke(&mut self, x: u64) {
        self.a.poke(x);
        *self.weight = self.weight.wrapping_add(x);
    }
}

impl<'g> Use for Three<'g> {
    fn use_it(&self, x: u64) -> u64 {
        self.a.use_it(x).wrapping_add(self.weight)
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
    fn into_rebrand<'x>(self, to: Witness<'x>) -> Nested<'x> {
        Nested {
            name: self.name.into_rebrand(to),
            header: self.header.into_rebrand(to),
            rows: self.rows.into_rebrand(to),
            selected: self.selected.into_rebrand(to),
        }
    }
    fn from_restore<'x>(from: Nested<'x>, at: Witness<'x>) -> Self {
        Nested {
            name: Rebrand::from_restore(from.name, at),
            header: Rebrand::from_restore(from.header, at),
            rows: Rebrand::from_restore(from.rows, at),
            selected: Rebrand::from_restore(from.selected, at),
        }
    }
}

pub struct NestedMut<'a, 'g> {
    pub name: &'a mut String,
    pub header: ThreeMut<'a, 'g>,
    pub rows: ListMut<'a, Three<'g>>,
    pub selected: OptionMut<'a, Cell<'g, u64>>,
}

impl<'g> BorrowMut for Nested<'g> {
    type Mut<'a>
        = NestedMut<'a, 'g>
    where
        Self: 'a;
    fn borrow_mut<'a>(from: &'a mut Nested<'static>) -> NestedMut<'a, 'g>
    where
        Self: 'a,
    {
        NestedMut {
            name: String::borrow_mut(&mut from.name),
            header: Three::borrow_mut(&mut from.header),
            rows: Vec::<Three<'g>>::borrow_mut(&mut from.rows),
            selected: Option::<Cell<'g, u64>>::borrow_mut(&mut from.selected),
        }
    }
}

impl<'g> Poke for Nested<'g> {
    fn poke(&mut self, x: u64) {
        self.name.poke(x);
        self.header.poke(x);
        self.rows.poke(x);
        self.selected.poke(x);
    }
}

impl<'a, 'g> Poke for NestedMut<'a, 'g> {
    fn poke(&mut self, x: u64) {
        self.name.poke(x);
        self.header.poke(x);
        self.rows.poke(x);
        self.selected.poke(x);
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

fn three<'g>(k: u64) -> Three<'g> {
    Three {
        a: Cell::new(3 * k),
        b: Cell::new(3 * k + 1),
        c: Cell::new(3 * k + 2),
        weight: k * 10,
        count: k as u32,
        flags: 0b101,
    }
}

/// A family of shapes, one type per brand, so the engine can name the shape
/// at the brand a closure was written at. `make(x)` is what an upstream map
/// builds from event `x`.
pub trait Family: 'static {
    type At<'g>: BorrowMut + Poke + Use;
    fn make<'g>(x: u64) -> Self::At<'g>;
}

pub struct U64s;
pub struct Threes;
pub struct Vec10;
pub struct Vec1000;
pub struct Nesteds;

impl Family for U64s {
    type At<'g> = u64;
    fn make<'g>(x: u64) -> Self::At<'g> {
        x
    }
}

impl Family for Threes {
    type At<'g> = Three<'g>;
    fn make<'g>(x: u64) -> Three<'g> {
        three(x)
    }
}

impl Family for Vec10 {
    type At<'g> = Vec<Cell<'g, u64>>;
    fn make<'g>(x: u64) -> Self::At<'g> {
        (0..10).map(|i| Cell::new(x + i)).collect()
    }
}

impl Family for Vec1000 {
    type At<'g> = Vec<Cell<'g, u64>>;
    fn make<'g>(x: u64) -> Self::At<'g> {
        (0..1000).map(|i| Cell::new(x + i)).collect()
    }
}

impl Family for Nesteds {
    type At<'g> = Nested<'g>;
    fn make<'g>(x: u64) -> Nested<'g> {
        Nested {
            name: String::from("inventory panel"),
            header: three(x),
            rows: (1..=4).map(|k| three(x + k)).collect(),
            selected: Some(Cell::new(x + 7)),
        }
    }
}

// ---------------------------------------------------------------------------
// The five writes

/// A stored value: `None` only while `owned` has taken it out.
type Slot<T> = Option<<T as Rebrand>::Of<'static>>;

fn slot_mut<T: Rebrand>(stored: &mut dyn Any) -> &mut Slot<T> {
    stored
        .downcast_mut::<Slot<T>>()
        .expect("a stored value has the type its node was built with")
}

fn slot_ref<T: Rebrand>(stored: &dyn Any) -> &T::Of<'static> {
    stored
        .downcast_ref::<Slot<T>>()
        .expect("a stored value has the type its node was built with")
        .as_ref()
        .expect("a stored value is in its slot between writes")
}

/// One way to store a value written at some brand, and to run
/// `accumulate_mut`'s `f` on a stored state. `black_box` on what `f` gets
/// keeps the optimizer from skipping the copy, which it can't do once `f` is
/// the user's, behind a `dyn`.
pub trait Writer: 'static {
    fn store<T: BorrowMut>(stored: &mut dyn Any, value: T);
    fn update<T: BorrowMut + Poke>(stored: &mut dyn Any, x: u64);
}

/// gc-arena's route: a cast.
pub struct Transmute;
/// The brand-erasure fixture's `store`, and its `accumulate_mut`.
pub struct ByRef;
/// A move for a brand-free type, `ByRef` otherwise.
pub struct View;
/// Owned rebrand, and an owned round trip.
pub struct Owned;
/// A derived mutable view, for `update` only.
pub struct Borrowed;

impl Writer for Transmute {
    fn store<T: BorrowMut>(stored: &mut dyn Any, value: T) {
        let value = ManuallyDrop::new(value);
        // SAFETY (experiment only; Bough forbids `unsafe`): `T::Of<'static>`
        // and `T` are one type up to lifetimes, so they share a layout, and
        // `value` is never dropped, so the read moves it.
        let value = unsafe { std::ptr::read((&*value as *const T).cast::<T::Of<'static>>()) };
        *slot_mut::<T>(stored) = Some(value);
    }

    fn update<T: BorrowMut + Poke>(stored: &mut dyn Any, x: u64) {
        let state: *mut T::Of<'static> = slot_mut::<T>(stored)
            .as_mut()
            .expect("the state is in its slot");
        // SAFETY (experiment only): as above; the borrow ends with the call.
        let state: &mut T = unsafe { &mut *state.cast::<T>() };
        black_box(state).poke(x);
    }
}

impl Writer for ByRef {
    fn store<T: BorrowMut>(stored: &mut dyn Any, value: T) {
        *slot_mut::<T>(stored) = Some(value.rebrand(erased()));
    }

    fn update<T: BorrowMut + Poke>(stored: &mut dyn Any, x: u64) {
        update_by_ref::<T>(slot_mut::<T>(stored), x);
    }
}

/// The fixture's round trip on a slot already downcast, so `View` falls back
/// to it without a second downcast.
fn update_by_ref<T: BorrowMut + Poke>(slot: &mut Slot<T>, x: u64) {
    let mut state = T::restore(slot.as_ref().expect("the state is in its slot"), erased());
    black_box(&mut state).poke(x);
    *slot = Some(state.rebrand(erased()));
}

impl Writer for View {
    fn store<T: BorrowMut>(stored: &mut dyn Any, value: T) {
        match value.into_static() {
            Ok(value) => *slot_mut::<T>(stored) = Some(value),
            Err(value) => ByRef::store(stored, value),
        }
    }

    fn update<T: BorrowMut + Poke>(stored: &mut dyn Any, x: u64) {
        let slot = slot_mut::<T>(stored);
        match T::view_mut(slot.as_mut().expect("the state is in its slot")) {
            Some(state) => black_box(state).poke(x),
            None => update_by_ref::<T>(slot, x),
        }
    }
}

impl Writer for Owned {
    fn store<T: BorrowMut>(stored: &mut dyn Any, value: T) {
        *slot_mut::<T>(stored) = Some(value.into_rebrand(erased()));
    }

    fn update<T: BorrowMut + Poke>(stored: &mut dyn Any, x: u64) {
        let slot = slot_mut::<T>(stored);
        let state = slot.take().expect("the state is in its slot");
        let mut state = T::from_restore(state, erased());
        black_box(&mut state).poke(x);
        *slot = Some(state.into_rebrand(erased()));
    }
}

impl Writer for Borrowed {
    /// A whole new value can't be written through a view of the old one
    /// without rebranding it, so a store is `Owned`'s.
    fn store<T: BorrowMut>(stored: &mut dyn Any, value: T) {
        Owned::store(stored, value);
    }

    fn update<T: BorrowMut + Poke>(stored: &mut dyn Any, x: u64) {
        let state = slot_mut::<T>(stored)
            .as_mut()
            .expect("the state is in its slot");
        let mut view = T::borrow_mut(state);
        black_box(&mut view).poke(x);
    }
}

/// The switch's read, the same in every variant: the cast, which the read
/// probe found costs what a derived borrowed view does.
fn read_cast<T: Rebrand + Use>(stored: &dyn Any, x: u64) -> u64 {
    let stored: *const T::Of<'static> = slot_ref::<T>(stored);
    // SAFETY (experiment only): as in `Transmute`.
    let value: &T = unsafe { &*stored.cast::<T>() };
    black_box(value).use_it(x)
}

/// The safe read, for the checksum only.
fn read_restored<T: Rebrand + Use>(stored: &dyn Any, x: u64) -> u64 {
    T::restore(slot_ref::<T>(stored), erased()).use_it(x)
}

// Each runs its body at a brand `'g` the compiler picks, as a closure written
// inside a `mutate` does; lifetimes are erased before codegen.

fn store_at<W: Writer, F: Family>(stored: &mut dyn Any, x: u64) {
    fn at<'g, W: Writer, F: Family>(stored: &mut dyn Any, x: u64, _: Witness<'g>) {
        W::store::<F::At<'g>>(stored, F::make(x));
    }
    at::<W, F>(stored, x, Witness::conjure())
}

fn update_at<W: Writer, F: Family>(stored: &mut dyn Any, x: u64) {
    fn at<'g, W: Writer, F: Family>(stored: &mut dyn Any, x: u64, _: Witness<'g>) {
        W::update::<F::At<'g>>(stored, x);
    }
    at::<W, F>(stored, x, Witness::conjure())
}

fn select_at<W: Writer, F: Family>(stored: &mut dyn Any, x: u64) {
    fn at<'g, W: Writer, F: Family>(stored: &mut dyn Any, x: u64, _: Witness<'g>) {
        let inner: Cell<'g, F::At<'g>> = Cell::new(INNER_A as u64 + (x & 1));
        W::store(stored, inner);
    }
    at::<W, F>(stored, x, Witness::conjure())
}

fn switch_read<F: Family>(cells: &[Box<dyn Any>], x: u64) -> u64 {
    fn at<'g, F: Family>(cells: &[Box<dyn Any>], x: u64, _: Witness<'g>) -> u64 {
        let outer = slot_ref::<Cell<'g, F::At<'g>>>(&*cells[OUTER]);
        let inner = outer.id.index as usize;
        read_cast::<F::At<'g>>(&*cells[inner], x)
    }
    at::<F>(cells, x, Witness::conjure())
}

fn checksum_at<F: Family>(stored: &dyn Any) -> u64 {
    fn at<'g, F: Family>(stored: &dyn Any, _: Witness<'g>) -> u64 {
        read_restored::<F::At<'g>>(stored, 3)
    }
    at::<F>(stored, Witness::conjure())
}

// ---------------------------------------------------------------------------
// The propagation

/// The cells: what `store`, `update`, `hold` and `accumulate` write (the
/// hold's cell or the state), node 20's fired event, and for `switch_cell`
/// the outer hold, its pending selection and the two inners it selects.
const HELD: usize = 0;
const FIRED: usize = 1;
const OUTER: usize = 2;
const PENDING: usize = 3;
const INNER_A: usize = 4;

type Cells = Vec<Box<dyn Any>>;
type Eval = Box<dyn Fn(&mut Cells, u64) -> u64>;
type Commit = Box<dyn Fn(&mut Cells, u64)>;

struct Node {
    inputs: Vec<usize>,
    dependents: Vec<usize>,
    eval: Eval,
}

/// The node under test.
const PROBE: usize = 20;

/// 22 nodes in rank order, the read probe's: an input, four chains of four
/// maps, three merges, the node under test (or a map), a last node (a hold
/// of the event, or the switch). The node under test may write cells, and
/// `commit` runs what the engine does at commit with its event.
struct Net {
    nodes: Vec<Node>,
    fired: Vec<Option<u64>>,
    queued: Vec<bool>,
    heap: BinaryHeap<Reverse<usize>>,
    touched: Vec<usize>,
    cells: Cells,
    commit: Option<Commit>,
    held: u64,
}

impl Net {
    fn new(cells: Cells, probe: Option<Eval>, last: Option<Eval>, commit: Option<Commit>) -> Self {
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
        let last = last.unwrap_or_else(|| Box::new(|_, e| e));
        specs.push((vec![PROBE], last));

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
            commit,
            held: 0,
        }
    }

    /// One transaction: fire the input, evaluate in rank order, commit.
    fn send(&mut self, x: u64) {
        self.fired[0] = Some(x);
        self.touched.push(0);
        self.enqueue(0);
        while let Some(Reverse(i)) = self.heap.pop() {
            // A merge combines what fired; every other node has one input.
            let event = self.nodes[i]
                .inputs
                .iter()
                .filter_map(|&j| self.fired[j])
                .fold(0u64, |a, e| a.wrapping_add(e));
            let out = (self.nodes[i].eval)(&mut self.cells, event);
            self.fired[i] = Some(out);
            self.touched.push(i);
            self.enqueue(i);
        }
        // Commit: the node under test's write lands, the last node's event
        // is held, streams clear.
        if let (Some(commit), Some(e)) = (&self.commit, self.fired[PROBE]) {
            commit(&mut self.cells, e);
        }
        let last = self.nodes.len() - 1;
        if let Some(v) = self.fired[last] {
            self.held = self.held.wrapping_add(v);
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
    Rebrand,
    View,
    Owned,
    Borrow,
}

impl Variant {
    pub const ALL: [Variant; 5] = [
        Variant::Baseline,
        Variant::Rebrand,
        Variant::View,
        Variant::Owned,
        Variant::Borrow,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Variant::Baseline => "baseline",
            Variant::Rebrand => "rebrand",
            Variant::View => "view",
            Variant::Owned => "owned",
            Variant::Borrow => "borrow",
        }
    }

    /// `borrow` is a way to mutate, so it is measured on updates only.
    pub fn applies(self, op: Op) -> bool {
        self != Variant::Borrow || matches!(op, Op::Update | Op::Accumulate)
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
    Store,
    Update,
    Hold,
    Accumulate,
    SwitchCell,
}

impl Op {
    pub const ALL: [Op; 5] = [
        Op::Store,
        Op::Update,
        Op::Hold,
        Op::Accumulate,
        Op::SwitchCell,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Op::Store => "store",
            Op::Update => "update",
            Op::Hold => "hold",
            Op::Accumulate => "accumulate",
            Op::SwitchCell => "switch_cell",
        }
    }
}

trait Run {
    fn run(&mut self, n: u64) -> u64;
    fn checksum(&self) -> u64;
}

struct Harness<W, F> {
    op: Op,
    net: Net,
    x: u64,
    kinds: PhantomData<fn() -> (W, F)>,
}

/// The initial cells, every value stored at `'static` by the safe route.
fn cells<F: Family>() -> Cells {
    fn at<'g, F: Family>(_: Witness<'g>) -> Cells {
        fn slot<T: Rebrand>(value: Option<T>) -> Box<dyn Any> {
            let value: Slot<T> = value.map(|v| v.into_rebrand(erased()));
            Box::new(value)
        }
        let outer: Cell<'g, F::At<'g>> = Cell::new(INNER_A as u64);
        vec![
            slot::<F::At<'g>>(Some(F::make(0))),
            slot::<F::At<'g>>(None),
            slot(Some(outer)),
            slot::<Cell<'g, F::At<'g>>>(None),
            slot::<F::At<'g>>(Some(F::make(1))),
            slot::<F::At<'g>>(Some(F::make(2))),
        ]
    }
    at::<F>(Witness::conjure())
}

/// Moves a stored value from one slot to another, as a hold's commit takes
/// the fired event: no rebrand, the same in every variant.
fn take_into<T: Rebrand>(cells: &mut Cells, from: usize, to: usize) {
    let value = slot_mut::<T>(&mut *cells[from]).take();
    *slot_mut::<T>(&mut *cells[to]) = value;
}

fn commit_hold<F: Family>(cells: &mut Cells, _: u64) {
    fn at<'g, F: Family>(cells: &mut Cells, _: Witness<'g>) {
        take_into::<F::At<'g>>(cells, FIRED, HELD);
    }
    at::<F>(cells, Witness::conjure())
}

fn commit_select<F: Family>(cells: &mut Cells, _: u64) {
    fn at<'g, F: Family>(cells: &mut Cells, _: Witness<'g>) {
        take_into::<Cell<'g, F::At<'g>>>(cells, PENDING, OUTER);
    }
    at::<F>(cells, Witness::conjure())
}

impl<W: Writer, F: Family> Harness<W, F> {
    fn new(op: Op) -> Self {
        let (probe, last, commit): (Option<Eval>, Option<Eval>, Option<Commit>) = match op {
            Op::Store | Op::Update => (None, None, None),
            Op::Hold => (
                Some(Box::new(|cells, e| {
                    store_at::<W, F>(&mut *cells[FIRED], e);
                    e
                })),
                None,
                Some(Box::new(commit_hold::<F>)),
            ),
            // `accumulate_mut`'s `f` runs at commit, on node 20's event.
            Op::Accumulate => (
                None,
                None,
                Some(Box::new(|cells, e| update_at::<W, F>(&mut *cells[HELD], e))),
            ),
            Op::SwitchCell => (
                Some(Box::new(|cells, e| {
                    select_at::<W, F>(&mut *cells[PENDING], e);
                    e
                })),
                Some(Box::new(|cells, e| switch_read::<F>(cells, e))),
                Some(Box::new(commit_select::<F>)),
            ),
        };
        Harness {
            op,
            net: Net::new(cells::<F>(), probe, last, commit),
            x: 0,
            kinds: PhantomData,
        }
    }
}

impl<W: Writer, F: Family> Run for Harness<W, F> {
    fn run(&mut self, n: u64) -> u64 {
        for _ in 0..n {
            let x = self.x;
            self.x += 1;
            match self.op {
                Op::Store => store_at::<W, F>(black_box(&mut *self.net.cells[HELD]), x),
                Op::Update => update_at::<W, F>(black_box(&mut *self.net.cells[HELD]), x),
                Op::Hold | Op::Accumulate | Op::SwitchCell => self.net.send(x),
            }
        }
        self.net.held
    }

    fn checksum(&self) -> u64 {
        let cell = if self.op == Op::SwitchCell {
            INNER_A
        } else {
            HELD
        };
        self.net
            .held
            .wrapping_add(checksum_at::<F>(&*self.net.cells[cell]))
    }
}

/// One (operation, variant, shape), built outside what is measured.
pub struct Probe {
    inner: Box<dyn Run>,
}

impl Probe {
    pub fn new(op: Op, variant: Variant, shape: Shape) -> Self {
        assert!(variant.applies(op), "{variant:?} is not measured on {op:?}");
        fn by_shape<W: Writer>(op: Op, shape: Shape) -> Box<dyn Run> {
            match shape {
                Shape::U64 => Box::new(Harness::<W, U64s>::new(op)),
                Shape::Three => Box::new(Harness::<W, Threes>::new(op)),
                Shape::Vec10 => Box::new(Harness::<W, Vec10>::new(op)),
                Shape::Vec1000 => Box::new(Harness::<W, Vec1000>::new(op)),
                Shape::Nested => Box::new(Harness::<W, Nesteds>::new(op)),
            }
        }
        let inner = match variant {
            Variant::Baseline => by_shape::<Transmute>(op, shape),
            Variant::Rebrand => by_shape::<ByRef>(op, shape),
            Variant::View => by_shape::<View>(op, shape),
            Variant::Owned => by_shape::<Owned>(op, shape),
            Variant::Borrow => by_shape::<Borrowed>(op, shape),
        };
        Probe { inner }
    }

    /// `n` writes for `store` and `update`, `n` events for the rest.
    pub fn run(&mut self, n: u64) -> u64 {
        self.inner.run(n)
    }

    /// The written value, read back by the safe route, and what the
    /// propagation held: the same for every variant. Not measured.
    pub fn checksum(&self) -> u64 {
        self.inner.checksum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant writes the same values, so the counts compare like work.
    #[test]
    fn variants_agree() {
        for op in Op::ALL {
            for shape in Shape::ALL {
                let sums: Vec<(u64, u64)> = Variant::ALL
                    .iter()
                    .filter(|v| v.applies(op))
                    .map(|&v| {
                        let mut p = Probe::new(op, v, shape);
                        (p.run(57), p.checksum())
                    })
                    .collect();
                assert!(
                    sums.windows(2).all(|w| w[0] == w[1]),
                    "{op:?} {shape:?} {sums:?}"
                );
            }
        }
    }

    /// The mutable view grows a token-bearing state in place, as a `Vec`
    /// accumulator's push, and hands tokens back at the writer's brand.
    #[test]
    fn list_mut_pushes_in_place() {
        fn at<'g>(_: Witness<'g>) {
            let mut stored: Vec<Cell<'static, u64>> = Vec::with_capacity(4);
            let ptr = stored.as_ptr();
            let mut view = Vec::<Cell<'g, u64>>::borrow_mut(&mut stored);
            for i in 0..4 {
                view.push(Cell::new(i));
            }
            let third: Cell<'g, u64> = view.get_mut(2).get();
            assert_eq!(third.id.index, 2);
            assert_eq!(stored.len(), 4);
            assert_eq!(stored.as_ptr(), ptr);
        }
        at(Witness::conjure());
    }

    /// The owned rebrand keeps a `Vec`'s buffer, so it allocates nothing.
    #[test]
    fn owned_rebrand_keeps_the_buffer() {
        let v: Vec<Cell<'_, u64>> = Vec1000::make(0);
        let ptr = v.as_ptr() as usize;
        let stored = v.into_rebrand(erased());
        assert_eq!(stored.as_ptr() as usize, ptr);
        let back: Vec<Cell<'_, u64>> = Rebrand::from_restore(stored, erased());
        assert_eq!(back.as_ptr() as usize, ptr);
    }
}
