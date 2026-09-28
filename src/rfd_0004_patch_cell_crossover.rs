//! At what collection size does a cell carrying deltas beat a cell of the
//! collection, for a `Vec` and for a keyed map?
//!
//! RFD 4's cell of a collection steps by value: an `accumulate_mut` applies
//! each event in place at commit (a `State<S>`, so no clone per step), and
//! the cells derived from it are read-through `map_cell`s, memoized in a
//! `OnceCell` that commit clears. So a derived cell recomputes from scratch
//! on the first read after every step, and not at all when nobody reads.
//! Open question 9's alternative is a cell whose step is a patch
//! (Maier's incremental lists, Reflex's `Incremental`, DBSP's Z-sets): the
//! integrate cell applies the patch, and the derived cells are stateful
//! nodes that update from the patch alone, every step, read or not.
//!
//! This is a model of both, no engine: one call to `step` is one instant,
//! the events the sources fire, the merge that composes them, the commit
//! that applies them and clears the memos, and the reader's read, if it
//! reads this instant. Marking, ordering and listeners are left out: both
//! designs pay them alike.
//!
//! - `Vecs`: a `Vec<u64>` with `map(f)` then `sum` downstream. An event is
//!   one of Maier's atoms, `Insert(i, x)` or `Remove(i)`. The baseline
//!   applies it to the vector, and the read maps the whole vector into a
//!   fresh memo and sums that. The delta design applies the atom to the
//!   vector, to the mapped vector, and to the sum. Inserts and removes
//!   alternate by instant, so the size stays at n or n + k and the fixture
//!   runs steadily.
//! - `Maps`: a `HashMap<u64, u64>` of n keys with a filter (even values)
//!   and a count downstream. An event is an upsert `(key, value)` of an
//!   existing key. The baseline inserts it, and the read collects the
//!   filtered map into a fresh memo and takes its length. The delta design
//!   turns the upsert into a Z-set, `{(key, old, -1), (key, new, +1)}`,
//!   reading the old value from the integral as a keyed-collection library
//!   would, then applies it to the map, to the filtered map and to the
//!   count.
//!
//! Both baselines and both delta designs keep the derived collection (the
//! mapped vector, the filtered map), not only the scalar at the end, since
//! a `map_cell`'s value is the collection and a reader of it expects one.
//!
//! A workload sets `k`, the number of sources that fire in each instant,
//! and `r`, the reader reading every r-th instant. With `k > 1` the merge
//! composes the k events into one: for `Vec` patches, concatenation (the
//! trace's indices are drawn against the vector as the earlier atoms leave
//! it, as Maier's non-commutative composition requires), and for Z-sets,
//! concatenation then consolidation (sort, add the weights of equal
//! elements, drop zeros), which a Z-set needs before it can be applied in
//! two passes, removals first. The baseline's merge concatenates commands.
//! The sources of one instant touch distinct keys; see `same_key_conflict`
//! for why.
//!
//! Two variants were added after the first run:
//!
//! - `rope`, for `Vecs`: the delta design over chunked sequences instead of
//!   flat vectors, for the source and for the mapped collection alike, each
//!   its own `Rope`. The flat delta design's insert and remove at an index
//!   move half of both vectors, so it is O(n) too; a rope makes them
//!   O(√n). Maier and Odersky's reactive sequences use a concatenation tree
//!   for the same reason.
//! - `lazy`, for `Maps`: the delta design with the derived cells deferred.
//!   The integral applies each instant's Z-set at once, since the next
//!   upsert's Z-set needs the old value from it, but the filter and count
//!   only append it to a pending buffer. A read, or the buffer reaching
//!   `LAZY_BOUND` elements, consolidates the buffer and applies it, so
//!   updates to one key between reads cancel down to one retraction and
//!   one insertion.
//!
//! Two more were added after the second run:
//!
//! - `btree`, for `Vecs`: the delta design over counted B-trees, whose
//!   internal nodes keep each child's size, so an insert or remove at an
//!   index is O(log n) with nodes of a fixed size, against the rope's O(√n)
//!   with a directory it scans. The benches take it and the rope to n of a
//!   million.
//! - `fullylazy`, for `Maps`: the source map deferred as well. A commit
//!   appends the raw upserts to a buffer, and a read, or the buffer reaching
//!   `FULLY_LAZY_BOUND` upserts, collapses each key to its last write and
//!   brings the source, the filter and the count up to date. `lazy` could
//!   not defer the source, because each Z-set's retraction needs the old
//!   value from it; raw upserts need nothing from it until the read.
//!
//! Two more were added after the third run, to separate what `fullylazy`
//! and `btree` won by from what they were built to test:
//!
//! - `fused`, for `Maps`: the delta design eager, every instant, but with
//!   `fullylazy`'s fusion: each upsert is one `insert` on the source, and
//!   the old value it hands back is the filter's retraction, so there is no
//!   lookup, no separate removal and no Z-set to consolidate. Against
//!   `delta` it measures the fusion; against `fullylazy`, the deferral.
//! - `btree-shared`, for `Vecs`: one counted B-tree whose leaves hold each
//!   source element beside its mapped value, so one descent serves the
//!   integral and the `map`, as a fused map node's would. Against `btree`,
//!   which keeps a tree per node, it measures what the second tree costs.

use std::collections::HashMap;

/// Instants in one cycle of a trace. The benches count one cycle.
pub const CYCLE: usize = 64;

/// The collection sizes the benches sweep.
pub const SIZES: [usize; 7] = [3, 10, 30, 100, 1_000, 10_000, 100_000];

/// The largest size, for `Vecs` only, where O(log n) and O(√n) part.
pub const MILLION: usize = 1_000_000;

/// The numbers of patches composed in one instant, for `Compose`.
pub const COMPOSE_K: [usize; 5] = [1, 2, 4, 16, 64];

/// How often the rare reader reads.
pub const RARE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// A cell of the collection, derived cells read-through.
    Baseline,
    /// A cell carrying deltas, derived cells incremental.
    Delta,
    /// `Vecs` only: `Delta` over ropes instead of flat vectors.
    Rope,
    /// `Maps` only: `Delta` with the derived cells' Z-sets buffered until a
    /// read.
    Lazy,
    /// `Vecs` only: `Delta` over counted B-trees.
    BTree,
    /// `Maps` only: the raw upserts buffered until a read, the source map
    /// included.
    FullyLazy,
    /// `Maps` only: `Delta` with each upsert one insert on the source, whose
    /// old value is the filter's retraction; eager, nothing buffered.
    Fused,
    /// `Vecs` only: `BTree` with one tree whose leaves hold the source and
    /// mapped values together.
    BTreeShared,
}

impl Variant {
    /// The variants every fixture has.
    pub const ALL: [Variant; 2] = [Variant::Baseline, Variant::Delta];
    /// The variants of `Vecs`.
    pub const VEC: [Variant; 5] = [
        Variant::Baseline,
        Variant::Delta,
        Variant::Rope,
        Variant::BTree,
        Variant::BTreeShared,
    ];
    /// The variants of `Maps`.
    pub const MAP: [Variant; 5] = [
        Variant::Baseline,
        Variant::Delta,
        Variant::Lazy,
        Variant::FullyLazy,
        Variant::Fused,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Variant::Baseline => "baseline",
            Variant::Delta => "delta",
            Variant::Rope => "rope",
            Variant::Lazy => "lazy",
            Variant::BTree => "btree",
            Variant::FullyLazy => "fullylazy",
            Variant::Fused => "fused",
            Variant::BTreeShared => "btree-shared",
        }
    }
}

/// Sources firing per instant, how often the reader reads, and, for
/// `Vec`, whether edits happen at the end rather than at a random index.
#[derive(Clone, Copy, Debug)]
pub struct Workload {
    pub name: &'static str,
    pub k: usize,
    pub read_every: usize,
    pub append: bool,
}

pub const ONE: Workload = Workload {
    name: "one",
    k: 1,
    read_every: 1,
    append: false,
};
pub const TWO: Workload = Workload {
    name: "two",
    k: 2,
    read_every: 1,
    append: false,
};
pub const RARE_READ: Workload = Workload {
    name: "rare",
    k: 1,
    read_every: RARE,
    append: false,
};
pub const APPEND: Workload = Workload {
    name: "append",
    k: 1,
    read_every: 1,
    append: true,
};

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// splitmix64.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

// ---- Vec ----

/// One of Maier's delta atoms. A patch is a sequence of them, composed
/// by concatenation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Insert(usize, u64),
    Remove(usize),
}

/// The element function of the `map`: cheap, as in Maier's crossover.
#[inline]
pub fn f(x: u64) -> u64 {
    x.wrapping_mul(0x9E37_79B9).wrapping_add(1)
}

fn apply_edit(v: &mut Vec<u64>, e: Edit) {
    match e {
        Edit::Insert(i, x) => v.insert(i, x),
        Edit::Remove(i) => {
            v.remove(i);
        }
    }
}

/// RFD 4's cell of a `Vec`: an `accumulate_mut` state and two read-through
/// cells over it, each a memo commit clears.
#[derive(Clone)]
struct BaseVec {
    source: Vec<u64>,
    mapped: Option<Vec<u64>>,
    sum: Option<u64>,
}

impl BaseVec {
    fn commit(&mut self, patch: &[Edit]) {
        for &e in patch {
            apply_edit(&mut self.source, e);
        }
        // The memos of the cells that stepped are cleared at commit.
        self.mapped = None;
        self.sum = None;
    }

    fn read(&mut self) -> u64 {
        if let Some(s) = self.sum {
            return s;
        }
        let source = &self.source;
        let mapped = self
            .mapped
            .get_or_insert_with(|| source.iter().map(|&x| f(x)).collect());
        let s = mapped.iter().fold(0u64, |a, &x| a.wrapping_add(x));
        self.sum = Some(s);
        s
    }
}

/// The patch-carrying cell: the integral, an incremental `map` that keeps
/// the mapped vector, and an incremental `sum` (a fold with an undo).
#[derive(Clone)]
struct DeltaVec {
    source: Vec<u64>,
    mapped: Vec<u64>,
    sum: u64,
}

impl DeltaVec {
    fn commit(&mut self, patch: &[Edit]) {
        for &e in patch {
            match e {
                Edit::Insert(i, x) => {
                    let y = f(x);
                    self.source.insert(i, x);
                    self.mapped.insert(i, y);
                    self.sum = self.sum.wrapping_add(y);
                }
                Edit::Remove(i) => {
                    self.source.remove(i);
                    let y = self.mapped.remove(i);
                    self.sum = self.sum.wrapping_sub(y);
                }
            }
        }
    }

    fn read(&self) -> u64 {
        self.sum
    }
}

/// A sequence in chunks of b to 2b elements (one chunk may be shorter),
/// with b about √n, and a directory of the chunks' lengths that an access
/// scans. An insert or remove at an index scans at most n/b lengths and
/// moves at most 2b elements, so it's O(√n) for the n the rope was built
/// for; a split or a merge moves the n/b chunk headers, also O(√n). A
/// counted B-tree, `BTree`, makes it O(log n).
#[derive(Clone)]
pub struct Rope {
    b: usize,
    lens: Vec<usize>,
    chunks: Vec<Vec<u64>>,
}

impl Rope {
    /// The chunk size for a rope of about n elements.
    pub fn chunk_size(n: usize) -> usize {
        n.isqrt().max(16)
    }

    pub fn new(items: &[u64], b: usize) -> Rope {
        assert!(b >= 2);
        let chunk = |c: &[u64]| {
            let mut v = Vec::with_capacity(2 * b + 1);
            v.extend_from_slice(c);
            v
        };
        let chunks: Vec<Vec<u64>> = if items.is_empty() {
            vec![chunk(&[])]
        } else {
            items.chunks(b).map(chunk).collect()
        };
        let lens = chunks.iter().map(Vec::len).collect();
        Rope { b, lens, chunks }
    }

    pub fn len(&self) -> usize {
        self.lens.iter().sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn to_vec(&self) -> Vec<u64> {
        self.chunks.concat()
    }

    /// The chunk holding index `i` and the offset in it. An insert may name
    /// the index one past a chunk's end, which appends to that chunk.
    fn locate(&self, mut i: usize, inserting: bool) -> (usize, usize) {
        for (c, &len) in self.lens.iter().enumerate() {
            if i < len || (inserting && i == len) {
                return (c, i);
            }
            i -= len;
        }
        panic!("index out of range");
    }

    fn split_if_long(&mut self, c: usize) {
        if self.lens[c] > 2 * self.b {
            let mut tail = Vec::with_capacity(2 * self.b + 1);
            tail.extend_from_slice(&self.chunks[c][self.b..]);
            self.chunks[c].truncate(self.b);
            self.lens[c] = self.b;
            self.lens.insert(c + 1, tail.len());
            self.chunks.insert(c + 1, tail);
        }
    }

    pub fn insert(&mut self, i: usize, x: u64) {
        let (c, off) = self.locate(i, true);
        self.chunks[c].insert(off, x);
        self.lens[c] += 1;
        self.split_if_long(c);
    }

    pub fn remove(&mut self, i: usize) -> u64 {
        let (c, off) = self.locate(i, false);
        let x = self.chunks[c].remove(off);
        self.lens[c] -= 1;
        if self.lens[c] < self.b / 2 && self.chunks.len() > 1 {
            // Merge into the chunk before, or the one after for the first,
            // and split again if that made it too long.
            let a = c.saturating_sub(1);
            let next = self.chunks.remove(a + 1);
            self.lens.remove(a + 1);
            self.chunks[a].extend_from_slice(&next);
            self.lens[a] += next.len();
            self.split_if_long(a);
        }
        x
    }
}

/// `DeltaVec` over ropes: the integral and the incremental `map` each keep
/// their own rope, and so each locate the index themselves, as two nodes
/// would.
#[derive(Clone)]
struct DeltaRope {
    source: Rope,
    mapped: Rope,
    sum: u64,
}

impl DeltaRope {
    fn commit(&mut self, patch: &[Edit]) {
        for &e in patch {
            match e {
                Edit::Insert(i, x) => {
                    let y = f(x);
                    self.source.insert(i, x);
                    self.mapped.insert(i, y);
                    self.sum = self.sum.wrapping_add(y);
                }
                Edit::Remove(i) => {
                    self.source.remove(i);
                    let y = self.mapped.remove(i);
                    self.sum = self.sum.wrapping_sub(y);
                }
            }
        }
    }

    fn read(&self) -> u64 {
        self.sum
    }
}

/// The most elements a B-tree leaf holds, and the most children an internal
/// node has. Both are fixed, whatever n is, as in a real engine: a leaf is
/// 512 bytes of elements, and an internal node's sizes fit a cache line or
/// two, so a lookup scans at most 16 sizes a level.
pub const LEAF_MAX: usize = 64;
pub const INNER_MAX: usize = 16;

/// A node of a counted B-tree: a leaf of elements, or an internal node with
/// the number of elements under each child, so an index finds its child by
/// scanning the sizes, O(log n) levels of at most `INNER_MAX` each.
///
/// Generic over the element, so `SharedBTree` can hold a source element and
/// its mapped value side by side in one tree of the same shape.
#[derive(Clone)]
enum Node<T> {
    Leaf(Vec<T>),
    Inner {
        sizes: Vec<usize>,
        kids: Vec<Node<T>>,
    },
}

impl<T: Copy> Node<T> {
    fn leaf(items: &[T]) -> Node<T> {
        let mut v = Vec::with_capacity(LEAF_MAX + 1);
        v.extend_from_slice(items);
        Node::Leaf(v)
    }

    fn inner(kids: Vec<Node<T>>) -> Node<T> {
        let mut sizes = Vec::with_capacity(INNER_MAX + 1);
        sizes.extend(kids.iter().map(Node::len));
        let mut k = Vec::with_capacity(INNER_MAX + 1);
        k.extend(kids);
        Node::Inner { sizes, kids: k }
    }

    fn len(&self) -> usize {
        match self {
            Node::Leaf(v) => v.len(),
            Node::Inner { sizes, .. } => sizes.iter().sum(),
        }
    }

    /// Entries in this node: elements of a leaf, children of an inner node.
    fn width(&self) -> usize {
        match self {
            Node::Leaf(v) => v.len(),
            Node::Inner { kids, .. } => kids.len(),
        }
    }

    fn max_width(&self) -> usize {
        match self {
            Node::Leaf(_) => LEAF_MAX,
            Node::Inner { .. } => INNER_MAX,
        }
    }

    /// Below this a node merges with a neighbour. A quarter of the maximum,
    /// so a merge that overflows splits into halves well above it, and a
    /// node doesn't merge and split again on alternate edits.
    fn min_width(&self) -> usize {
        self.max_width() / 4
    }

    /// Splits off the upper half as a new right sibling.
    fn split(&mut self) -> Node<T> {
        match self {
            Node::Leaf(v) => {
                let right = Node::leaf(&v[v.len() / 2..]);
                v.truncate(v.len() / 2);
                right
            }
            Node::Inner { sizes, kids } => {
                let half = kids.len() / 2;
                let mut rs = Vec::with_capacity(INNER_MAX + 1);
                rs.extend(sizes.drain(half..));
                let mut rk = Vec::with_capacity(INNER_MAX + 1);
                rk.extend(kids.drain(half..));
                Node::Inner {
                    sizes: rs,
                    kids: rk,
                }
            }
        }
    }

    /// Appends the entries of `right`, a sibling of the same height.
    fn absorb(&mut self, right: Node<T>) {
        match (self, right) {
            (Node::Leaf(v), Node::Leaf(r)) => v.extend_from_slice(&r),
            (
                Node::Inner { sizes, kids },
                Node::Inner {
                    sizes: rs,
                    kids: rk,
                },
            ) => {
                sizes.extend(rs);
                kids.extend(rk);
            }
            _ => unreachable!("siblings have the same height"),
        }
    }

    /// The child holding index `i` and the index within it. An insert may
    /// name the index one past a child's end, which appends to that child.
    fn locate(sizes: &[usize], mut i: usize, inserting: bool) -> (usize, usize) {
        for (c, &len) in sizes.iter().enumerate() {
            if i < len || (inserting && i == len) {
                return (c, i);
            }
            i -= len;
        }
        panic!("index out of range");
    }

    /// Inserts, and returns the new right sibling if this node split.
    fn insert(&mut self, i: usize, x: T) -> Option<Node<T>> {
        match self {
            Node::Leaf(v) => v.insert(i, x),
            Node::Inner { sizes, kids } => {
                let (c, off) = Self::locate(sizes, i, true);
                match kids[c].insert(off, x) {
                    None => sizes[c] += 1,
                    Some(right) => {
                        sizes[c] = kids[c].len();
                        sizes.insert(c + 1, right.len());
                        kids.insert(c + 1, right);
                    }
                }
            }
        }
        (self.width() > self.max_width()).then(|| self.split())
    }

    /// Removes; the caller fixes this node if it underflowed.
    fn remove(&mut self, i: usize) -> T {
        match self {
            Node::Leaf(v) => v.remove(i),
            Node::Inner { sizes, kids } => {
                let (c, off) = Self::locate(sizes, i, false);
                let x = kids[c].remove(off);
                sizes[c] -= 1;
                if kids[c].width() < kids[c].min_width() && kids.len() > 1 {
                    // Merge into the child before, or the one after for the
                    // first, and split again if that made it too wide.
                    let a = c.saturating_sub(1);
                    let next = kids.remove(a + 1);
                    sizes.remove(a + 1);
                    kids[a].absorb(next);
                    if kids[a].width() > kids[a].max_width() {
                        let right = kids[a].split();
                        sizes.insert(a + 1, right.len());
                        kids.insert(a + 1, right);
                    }
                    sizes[a] = kids[a].len();
                }
                x
            }
        }
    }
}

/// A sequence as a counted B-tree (an order-statistic tree): O(log n) per
/// insert or remove at an index, against the rope's O(√n). Built bulk, with
/// nodes three-quarters full, so the first edits don't all split.
#[derive(Clone)]
pub struct BTree<T = u64> {
    root: Node<T>,
}

impl<T: Copy> BTree<T> {
    pub fn new(items: &[T]) -> BTree<T> {
        let mut level: Vec<Node<T>> = if items.is_empty() {
            vec![Node::leaf(&[])]
        } else {
            items.chunks(LEAF_MAX * 3 / 4).map(Node::leaf).collect()
        };
        while level.len() > 1 {
            let mut up = Vec::with_capacity(level.len() / (INNER_MAX * 3 / 4) + 1);
            let mut it = level.into_iter().peekable();
            while it.peek().is_some() {
                up.push(Node::inner(it.by_ref().take(INNER_MAX * 3 / 4).collect()));
            }
            level = up;
        }
        BTree {
            root: level.pop().unwrap(),
        }
    }

    pub fn len(&self) -> usize {
        self.root.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn to_vec(&self) -> Vec<T> {
        fn walk<T: Copy>(n: &Node<T>, out: &mut Vec<T>) {
            match n {
                Node::Leaf(v) => out.extend_from_slice(v),
                Node::Inner { kids, .. } => kids.iter().for_each(|k| walk(k, out)),
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    pub fn insert(&mut self, i: usize, x: T) {
        if let Some(right) = self.root.insert(i, x) {
            let left = std::mem::replace(&mut self.root, Node::Leaf(Vec::new()));
            self.root = Node::inner(vec![left, right]);
        }
    }

    pub fn remove(&mut self, i: usize) -> T {
        let x = self.root.remove(i);
        if let Node::Inner { kids, .. } = &mut self.root
            && kids.len() == 1
        {
            self.root = kids.pop().unwrap();
        }
        x
    }

    /// Checks the tree's invariants: every size is its child's length, no
    /// node is wider than its maximum, and every leaf is at the same depth.
    #[cfg(test)]
    fn check(&self) {
        fn walk<T: Copy>(n: &Node<T>, depth: usize, leaves: &mut Vec<usize>) {
            assert!(n.width() <= n.max_width());
            match n {
                Node::Leaf(_) => leaves.push(depth),
                Node::Inner { sizes, kids } => {
                    assert_eq!(sizes.len(), kids.len());
                    for (s, k) in sizes.iter().zip(kids) {
                        assert_eq!(*s, k.len());
                        walk(k, depth + 1, leaves);
                    }
                }
            }
        }
        let mut leaves = Vec::new();
        walk(&self.root, 0, &mut leaves);
        assert!(leaves.iter().all(|&d| d == leaves[0]));
    }

    #[cfg(test)]
    fn height(&self) -> usize {
        let mut h = 0;
        let mut n = &self.root;
        while let Node::Inner { kids, .. } = n {
            n = &kids[0];
            h += 1;
        }
        h
    }
}

/// `DeltaVec` over counted B-trees, one for the source and one for the
/// mapped collection, as `DeltaRope` has two ropes.
#[derive(Clone)]
struct DeltaBTree {
    source: BTree,
    mapped: BTree,
    sum: u64,
}

impl DeltaBTree {
    fn commit(&mut self, patch: &[Edit]) {
        for &e in patch {
            match e {
                Edit::Insert(i, x) => {
                    let y = f(x);
                    self.source.insert(i, x);
                    self.mapped.insert(i, y);
                    self.sum = self.sum.wrapping_add(y);
                }
                Edit::Remove(i) => {
                    self.source.remove(i);
                    let y = self.mapped.remove(i);
                    self.sum = self.sum.wrapping_sub(y);
                }
            }
        }
    }

    fn read(&self) -> u64 {
        self.sum
    }
}

/// `DeltaBTree` with one tree for both collections: each leaf entry is a
/// source element and its mapped value, so an edit descends once where
/// `DeltaBTree` descends twice. The leaves keep `LEAF_MAX` entries, twice
/// the bytes of `DeltaBTree`'s, so the tree has the same shape and height
/// and the comparison is one descent against two, not a change of fan-out.
/// The mapped collection is still there to read, as the `.1` of each entry.
#[derive(Clone)]
struct SharedBTree {
    both: BTree<(u64, u64)>,
    sum: u64,
}

impl SharedBTree {
    fn commit(&mut self, patch: &[Edit]) {
        for &e in patch {
            match e {
                Edit::Insert(i, x) => {
                    let y = f(x);
                    self.both.insert(i, (x, y));
                    self.sum = self.sum.wrapping_add(y);
                }
                Edit::Remove(i) => {
                    let (_, y) = self.both.remove(i);
                    self.sum = self.sum.wrapping_sub(y);
                }
            }
        }
    }

    fn read(&self) -> u64 {
        self.sum
    }
}

/// A `Vec` fixture: a cycle of instants, each `k` single-atom events, and
/// both designs' state, each with its own place in the cycle.
#[derive(Clone)]
pub struct Vecs {
    pub n: usize,
    pub workload: Workload,
    trace: Vec<Vec<Edit>>,
    patch: Vec<Edit>,
    base: BaseVec,
    base_at: usize,
    delta: DeltaVec,
    delta_at: usize,
    rope: DeltaRope,
    rope_at: usize,
    btree: DeltaBTree,
    btree_at: usize,
    shared: SharedBTree,
    shared_at: usize,
}

impl Vecs {
    pub fn new(n: usize, workload: Workload) -> Vecs {
        let mut rng = Rng::new(0x5EED ^ n as u64 ^ ((workload.k as u64) << 40));
        let source: Vec<u64> = (0..n).map(|_| rng.next_u64()).collect();
        // Indices are drawn against the length the earlier atoms leave, so
        // the trace is valid applied in order and returns to n each cycle.
        let mut len = n;
        let mut trace = Vec::with_capacity(CYCLE);
        for t in 0..CYCLE {
            let mut instant = Vec::with_capacity(workload.k);
            for _ in 0..workload.k {
                let e = if t % 2 == 0 {
                    let i = if workload.append {
                        len
                    } else {
                        rng.below(len + 1)
                    };
                    len += 1;
                    Edit::Insert(i, rng.next_u64())
                } else {
                    len -= 1;
                    let i = if workload.append {
                        len
                    } else {
                        rng.below(len + 1)
                    };
                    Edit::Remove(i)
                };
                instant.push(e);
            }
            trace.push(instant);
        }
        assert_eq!(len, n);
        let mapped: Vec<u64> = source.iter().map(|&x| f(x)).collect();
        let sum = mapped.iter().fold(0u64, |a, &x| a.wrapping_add(x));
        let b = Rope::chunk_size(n);
        let rope = DeltaRope {
            source: Rope::new(&source, b),
            mapped: Rope::new(&mapped, b),
            sum,
        };
        let btree = DeltaBTree {
            source: BTree::new(&source),
            mapped: BTree::new(&mapped),
            sum,
        };
        let pairs: Vec<(u64, u64)> = source.iter().copied().zip(mapped.iter().copied()).collect();
        let shared = SharedBTree {
            both: BTree::new(&pairs),
            sum,
        };
        Vecs {
            n,
            workload,
            trace,
            patch: Vec::with_capacity(workload.k),
            base: BaseVec {
                source: source.clone(),
                mapped: None,
                sum: None,
            },
            base_at: 0,
            delta: DeltaVec {
                source,
                mapped,
                sum,
            },
            delta_at: 0,
            rope,
            rope_at: 0,
            btree,
            btree_at: 0,
            shared,
            shared_at: 0,
        }
    }

    /// One instant of one design. Returns what the reader read, or 0 when
    /// it didn't read.
    pub fn step(&mut self, v: Variant) -> u64 {
        let at = match v {
            Variant::Baseline => &mut self.base_at,
            Variant::Delta => &mut self.delta_at,
            Variant::Rope => &mut self.rope_at,
            Variant::BTree => &mut self.btree_at,
            Variant::BTreeShared => &mut self.shared_at,
            Variant::Lazy | Variant::FullyLazy | Variant::Fused => {
                panic!("`{}` is a variant of `Maps`", v.name())
            }
        };
        let t = *at % CYCLE;
        let reads = *at % self.workload.read_every == 0;
        *at += 1;
        // The merge: k events into one patch, by concatenation, the same for
        // both designs (the baseline's merged commands are the same list).
        self.patch.clear();
        self.patch.extend_from_slice(&self.trace[t]);
        match v {
            Variant::Baseline => {
                self.base.commit(&self.patch);
                if reads { self.base.read() } else { 0 }
            }
            Variant::Delta => {
                self.delta.commit(&self.patch);
                if reads { self.delta.read() } else { 0 }
            }
            Variant::Rope => {
                self.rope.commit(&self.patch);
                if reads { self.rope.read() } else { 0 }
            }
            Variant::BTree => {
                self.btree.commit(&self.patch);
                if reads { self.btree.read() } else { 0 }
            }
            Variant::BTreeShared => {
                self.shared.commit(&self.patch);
                if reads { self.shared.read() } else { 0 }
            }
            Variant::Lazy | Variant::FullyLazy | Variant::Fused => unreachable!(),
        }
    }

    pub fn steps(&mut self, v: Variant, count: usize) -> u64 {
        let mut acc = 0u64;
        for _ in 0..count {
            acc = acc.wrapping_add(self.step(v));
        }
        acc
    }
}

// ---- Map ----

/// An element of a Z-set over `(key, value)` pairs, with its weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Z {
    pub key: u64,
    pub value: u64,
    pub weight: i64,
}

/// The filter downstream of the map.
#[inline]
pub fn keep(v: u64) -> bool {
    v & 1 == 0
}

/// Composes Z-sets already concatenated in `z`: sorts, adds the weights of
/// equal elements and drops zeros. This is the group's `+`.
pub fn consolidate(z: &mut Vec<Z>) {
    z.sort_unstable_by_key(|e| (e.key, e.value));
    let mut w = 0;
    for r in 0..z.len() {
        if w > 0 && z[w - 1].key == z[r].key && z[w - 1].value == z[r].value {
            z[w - 1].weight += z[r].weight;
            if z[w - 1].weight == 0 {
                w -= 1;
            }
        } else {
            z[w] = z[r];
            w += 1;
        }
    }
    z.truncate(w);
}

/// Turns each upsert into a Z-set against `integral` and composes them into
/// `out`, consolidating when more than one source fired.
pub fn compose_upserts(integral: &HashMap<u64, u64>, upserts: &[(u64, u64)], out: &mut Vec<Z>) {
    out.clear();
    for &(key, value) in upserts {
        if let Some(&old) = integral.get(&key) {
            out.push(Z {
                key,
                value: old,
                weight: -1,
            });
        }
        out.push(Z {
            key,
            value,
            weight: 1,
        });
    }
    if upserts.len() > 1 {
        consolidate(out);
    }
}

#[derive(Clone)]
struct BaseMap {
    source: HashMap<u64, u64>,
    filtered: Option<HashMap<u64, u64>>,
    count: Option<u64>,
}

impl BaseMap {
    fn commit(&mut self, upserts: &[(u64, u64)]) {
        for &(k, v) in upserts {
            self.source.insert(k, v);
        }
        self.filtered = None;
        self.count = None;
    }

    fn read(&mut self) -> u64 {
        if let Some(c) = self.count {
            return c;
        }
        let source = &self.source;
        let filtered = self.filtered.get_or_insert_with(|| {
            source
                .iter()
                .filter(|&(_, &v)| keep(v))
                .map(|(&k, &v)| (k, v))
                .collect()
        });
        let c = filtered.len() as u64;
        self.count = Some(c);
        c
    }
}

#[derive(Clone)]
struct DeltaMap {
    source: HashMap<u64, u64>,
    filtered: HashMap<u64, u64>,
    count: u64,
    zset: Vec<Z>,
}

impl DeltaMap {
    fn commit(&mut self, upserts: &[(u64, u64)]) {
        compose_upserts(&self.source, upserts, &mut self.zset);
        // Removals first, so an update's two halves apply in either order
        // of the sort.
        for pass in [-1i64, 1] {
            for z in &self.zset {
                if z.weight.signum() != pass {
                    continue;
                }
                let kept = keep(z.value);
                if pass < 0 {
                    self.source.remove(&z.key);
                    if kept {
                        self.filtered.remove(&z.key);
                        self.count -= 1;
                    }
                } else {
                    self.source.insert(z.key, z.value);
                    if kept {
                        self.filtered.insert(z.key, z.value);
                        self.count += 1;
                    }
                }
            }
        }
    }

    fn read(&self) -> u64 {
        self.count
    }
}

/// The most Z-set elements `LazyMap` holds pending before it applies them
/// without a read: 1,024, two per upsert, so 512 upserts. A fixed bound
/// keeps the buffer's memory and the pause of one flush constant whatever
/// n is; a bound in proportion to n would bound the buffer to the
/// collection's own size instead. No bench reaches it: the rare reader
/// leaves at most 32 elements pending.
pub const LAZY_BOUND: usize = 1_024;

/// `DeltaMap` with the derived cells lazy. The integral applies each
/// instant's Z-set at once, exactly as `DeltaMap` does, because the next
/// upsert's retraction reads the old value from it. The filter and the
/// count append the Z-set to `pending`, and apply the consolidated buffer
/// on a read or at the bound. Consolidating across instants is what's new:
/// a key updated from a to b to c between reads leaves `(a, -1), (c, +1)`.
#[derive(Clone)]
struct LazyMap {
    source: HashMap<u64, u64>,
    filtered: HashMap<u64, u64>,
    count: u64,
    zset: Vec<Z>,
    pending: Vec<Z>,
    /// Instants in `pending`; one instant's Z-set is consolidated already.
    instants: usize,
    bound: usize,
}

impl LazyMap {
    fn commit(&mut self, upserts: &[(u64, u64)]) {
        compose_upserts(&self.source, upserts, &mut self.zset);
        for pass in [-1i64, 1] {
            for z in &self.zset {
                if z.weight.signum() != pass {
                    continue;
                }
                if pass < 0 {
                    self.source.remove(&z.key);
                } else {
                    self.source.insert(z.key, z.value);
                }
            }
        }
        self.pending.extend_from_slice(&self.zset);
        self.instants += 1;
        if self.pending.len() >= self.bound {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.instants > 1 {
            consolidate(&mut self.pending);
        }
        // Removals first, as in `DeltaMap`: after consolidation a key has at
        // most one of each, and its removal must not take out its insertion.
        for pass in [-1i64, 1] {
            for z in &self.pending {
                if z.weight.signum() != pass || !keep(z.value) {
                    continue;
                }
                if pass < 0 {
                    self.filtered.remove(&z.key);
                    self.count -= 1;
                } else {
                    self.filtered.insert(z.key, z.value);
                    self.count += 1;
                }
            }
        }
        self.pending.clear();
        self.instants = 0;
    }

    fn read(&mut self) -> u64 {
        self.flush();
        self.count
    }
}

/// One upsert through the source, the filter and the count, fused: the
/// source's `insert` hands back the old value, which is the retraction the
/// filter needs, so the upsert costs one hash insert on the source and at
/// most a removal and an insertion on the filtered map. `FullyLazyMap` runs
/// this on each surviving upsert of a flush, `FusedMap` on each upsert of
/// each instant, so the two differ only in when.
#[inline]
fn fused_upsert(
    source: &mut HashMap<u64, u64>,
    filtered: &mut HashMap<u64, u64>,
    count: &mut u64,
    key: u64,
    value: u64,
) {
    let old = source.insert(key, value);
    if old == Some(value) {
        return;
    }
    if let Some(old) = old
        && keep(old)
    {
        filtered.remove(&key);
        *count -= 1;
    }
    if keep(value) {
        filtered.insert(key, value);
        *count += 1;
    }
}

/// The most raw upserts `FullyLazyMap` holds before it applies them without
/// a read: as many upserts as `LAZY_BOUND` holds, two Z-set elements each.
pub const FULLY_LAZY_BOUND: usize = LAZY_BOUND / 2;

/// A map whose source is lazy too. A commit only appends the instant's raw
/// upserts to `pending`; nothing is looked up. A read, or `pending` reaching
/// the bound, brings the source, the filter and the count up to date:
/// duplicate keys first collapse to their last write, since upserts to one
/// key compose to the last, then each surviving upsert writes the source
/// once, and the old value that write hands back is the retraction the
/// filter needs, so no Z-set is built or sorted. The one sort is of the
/// buffer by key, to find the duplicates.
///
/// With a read every instant this is an eager design too, one that fuses
/// `DeltaMap`'s lookup, removal and insertion into one insert. So `one` and
/// `two` measure that fusion, and `rare` adds what deferring saves.
#[derive(Clone)]
struct FullyLazyMap {
    source: HashMap<u64, u64>,
    filtered: HashMap<u64, u64>,
    count: u64,
    /// Upserts in arrival order, with their place in it, so a sort by key
    /// then place leaves the last write of each key at the end of its run.
    pending: Vec<(u64, u32, u64)>,
    bound: usize,
}

impl FullyLazyMap {
    fn commit(&mut self, upserts: &[(u64, u64)]) {
        for &(key, value) in upserts {
            let at = self.pending.len() as u32;
            self.pending.push((key, at, value));
        }
        if self.pending.len() >= self.bound {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.pending.len() > 1 {
            self.pending.sort_unstable_by_key(|&(key, at, _)| (key, at));
        }
        for (i, &(key, _, value)) in self.pending.iter().enumerate() {
            // A later write to the same key supersedes this one.
            if self.pending.get(i + 1).is_some_and(|&(k, _, _)| k == key) {
                continue;
            }
            fused_upsert(
                &mut self.source,
                &mut self.filtered,
                &mut self.count,
                key,
                value,
            );
        }
        self.pending.clear();
    }

    fn read(&mut self) -> u64 {
        self.flush();
        self.count
    }
}

/// `DeltaMap` with `FullyLazyMap`'s fusion and none of its deferral: every
/// instant, each upsert goes through `fused_upsert` at once. The Z-set the
/// source hands the filter, `{(key, old, -1), (key, new, +1)}`, is never
/// built as a vector: it is the pair `insert` returns and the upsert holds.
/// A composed instant needs no consolidation, since the sources of one
/// instant touch distinct keys, and applied in turn, upserts to one key
/// would compose to the last anyway.
#[derive(Clone)]
struct FusedMap {
    source: HashMap<u64, u64>,
    filtered: HashMap<u64, u64>,
    count: u64,
}

impl FusedMap {
    fn commit(&mut self, upserts: &[(u64, u64)]) {
        for &(key, value) in upserts {
            fused_upsert(
                &mut self.source,
                &mut self.filtered,
                &mut self.count,
                key,
                value,
            );
        }
    }

    fn read(&self) -> u64 {
        self.count
    }
}

/// A map fixture: a cycle of instants, each `k` upserts of distinct
/// existing keys, and both designs' state.
#[derive(Clone)]
pub struct Maps {
    pub n: usize,
    pub workload: Workload,
    trace: Vec<Vec<(u64, u64)>>,
    upserts: Vec<(u64, u64)>,
    base: BaseMap,
    base_at: usize,
    delta: DeltaMap,
    delta_at: usize,
    lazy: LazyMap,
    lazy_at: usize,
    fully: FullyLazyMap,
    fully_at: usize,
    fused: FusedMap,
    fused_at: usize,
}

impl Maps {
    pub fn new(n: usize, workload: Workload) -> Maps {
        assert!(
            n >= workload.k,
            "the sources of one instant need distinct keys"
        );
        let mut rng = Rng::new(0xC0FFEE ^ n as u64 ^ ((workload.k as u64) << 40));
        // Keys spread over u64, so hashing sees realistic keys.
        let keys: Vec<u64> = (0..n).map(|_| rng.next_u64()).collect();
        let source: HashMap<u64, u64> = keys.iter().map(|&k| (k, rng.next_u64())).collect();
        let mut trace = Vec::with_capacity(CYCLE);
        for _ in 0..CYCLE {
            let mut instant: Vec<(u64, u64)> = Vec::with_capacity(workload.k);
            while instant.len() < workload.k {
                let k = keys[rng.below(n)];
                if instant.iter().all(|&(k2, _)| k2 != k) {
                    instant.push((k, rng.next_u64()));
                }
            }
            trace.push(instant);
        }
        let filtered: HashMap<u64, u64> = source
            .iter()
            .filter(|&(_, &v)| keep(v))
            .map(|(&k, &v)| (k, v))
            .collect();
        let count = filtered.len() as u64;
        let fused = FusedMap {
            source: source.clone(),
            filtered: filtered.clone(),
            count,
        };
        let fully = FullyLazyMap {
            source: source.clone(),
            filtered: filtered.clone(),
            count,
            pending: Vec::with_capacity(FULLY_LAZY_BOUND + workload.k),
            bound: FULLY_LAZY_BOUND,
        };
        let lazy = LazyMap {
            source: source.clone(),
            filtered: filtered.clone(),
            count,
            zset: Vec::with_capacity(2 * workload.k),
            pending: Vec::with_capacity(LAZY_BOUND + 2 * workload.k),
            instants: 0,
            bound: LAZY_BOUND,
        };
        Maps {
            n,
            workload,
            trace,
            upserts: Vec::with_capacity(workload.k),
            base: BaseMap {
                source: source.clone(),
                filtered: None,
                count: None,
            },
            base_at: 0,
            delta: DeltaMap {
                source,
                filtered,
                count,
                zset: Vec::with_capacity(2 * workload.k),
            },
            delta_at: 0,
            lazy,
            lazy_at: 0,
            fully,
            fully_at: 0,
            fused,
            fused_at: 0,
        }
    }

    pub fn step(&mut self, v: Variant) -> u64 {
        let at = match v {
            Variant::Baseline => &mut self.base_at,
            Variant::Delta => &mut self.delta_at,
            Variant::Lazy => &mut self.lazy_at,
            Variant::FullyLazy => &mut self.fully_at,
            Variant::Fused => &mut self.fused_at,
            Variant::Rope | Variant::BTree | Variant::BTreeShared => {
                panic!("`{}` is a variant of `Vecs`", v.name())
            }
        };
        let t = *at % CYCLE;
        let cycle = (*at / CYCLE) as u64;
        let reads = *at % self.workload.read_every == 0;
        *at += 1;
        // The values change from cycle to cycle, or from the second cycle
        // on every upsert would write the value already there, and a
        // composed Z-set would cancel to nothing.
        let salt = cycle.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.upserts.clear();
        self.upserts.extend(
            self.trace[t]
                .iter()
                .map(|&(k, x)| (k, x.wrapping_add(salt))),
        );
        match v {
            Variant::Baseline => {
                self.base.commit(&self.upserts);
                if reads { self.base.read() } else { 0 }
            }
            Variant::Delta => {
                self.delta.commit(&self.upserts);
                if reads { self.delta.read() } else { 0 }
            }
            Variant::Lazy => {
                self.lazy.commit(&self.upserts);
                if reads { self.lazy.read() } else { 0 }
            }
            Variant::FullyLazy => {
                self.fully.commit(&self.upserts);
                if reads { self.fully.read() } else { 0 }
            }
            Variant::Fused => {
                self.fused.commit(&self.upserts);
                if reads { self.fused.read() } else { 0 }
            }
            Variant::Rope | Variant::BTree | Variant::BTreeShared => unreachable!(),
        }
    }

    pub fn steps(&mut self, v: Variant, count: usize) -> u64 {
        let mut acc = 0u64;
        for _ in 0..count {
            acc = acc.wrapping_add(self.step(v));
        }
        acc
    }
}

// ---- Composition alone ----

/// The cost of composing k patches in one instant, apart from applying
/// them: k single-source Z-sets (an update each) concatenated and
/// consolidated, against the baseline's merge, which concatenates k
/// commands. Composing `Vec` patches is concatenation too, so it costs
/// what the baseline's merge does.
#[derive(Clone)]
pub struct Compose {
    pub k: usize,
    upserts: Vec<(u64, u64)>,
    zsets: Vec<[Z; 2]>,
    commands: Vec<(u64, u64)>,
    out: Vec<Z>,
}

impl Compose {
    pub fn new(k: usize) -> Compose {
        let mut rng = Rng::new(0xFACE ^ k as u64);
        let upserts: Vec<(u64, u64)> = (0..k).map(|_| (rng.next_u64(), rng.next_u64())).collect();
        let zsets = upserts
            .iter()
            .map(|&(key, value)| {
                [
                    Z {
                        key,
                        value: rng.next_u64(),
                        weight: -1,
                    },
                    Z {
                        key,
                        value,
                        weight: 1,
                    },
                ]
            })
            .collect();
        Compose {
            k,
            upserts,
            zsets,
            commands: Vec::with_capacity(k),
            out: Vec::with_capacity(2 * k),
        }
    }

    pub fn run(&mut self, v: Variant) -> usize {
        match v {
            Variant::Baseline => {
                self.commands.clear();
                self.commands.extend_from_slice(&self.upserts);
                self.commands.len()
            }
            Variant::Delta => {
                self.out.clear();
                for z in &self.zsets {
                    self.out.extend_from_slice(z);
                }
                if self.k > 1 {
                    consolidate(&mut self.out);
                }
                self.out.len()
            }
            Variant::Rope
            | Variant::Lazy
            | Variant::BTree
            | Variant::FullyLazy
            | Variant::Fused
            | Variant::BTreeShared => {
                panic!("`compose` has no {} variant", v.name())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All designs read the same values at every instant the reader reads.
    #[test]
    fn designs_agree() {
        for n in [3, 10, 100, 1_000, 10_000] {
            for w in [ONE, TWO, RARE_READ, APPEND] {
                let mut a = Vecs::new(n, w);
                for _ in 0..3 * CYCLE {
                    let read = a.step(Variant::Baseline);
                    assert_eq!(read, a.step(Variant::Delta));
                    assert_eq!(read, a.step(Variant::Rope));
                    assert_eq!(read, a.step(Variant::BTree));
                    assert_eq!(read, a.step(Variant::BTreeShared));
                }
                assert_eq!(a.base.source, a.delta.source);
                assert_eq!(a.base.source, a.rope.source.to_vec());
                assert_eq!(a.delta.mapped, a.rope.mapped.to_vec());
                assert_eq!(a.base.source, a.btree.source.to_vec());
                assert_eq!(a.delta.mapped, a.btree.mapped.to_vec());
                a.btree.source.check();
                a.btree.mapped.check();
                let (source, mapped): (Vec<u64>, Vec<u64>) =
                    a.shared.both.to_vec().into_iter().unzip();
                assert_eq!(a.base.source, source);
                assert_eq!(a.delta.mapped, mapped);
                a.shared.both.check();
                assert_eq!(a.shared.both.height(), a.btree.source.height());
                if w.append {
                    continue;
                }
                let mut m = Maps::new(n, w);
                for _ in 0..3 * CYCLE {
                    let read = m.step(Variant::Baseline);
                    assert_eq!(read, m.step(Variant::Delta));
                    assert_eq!(read, m.step(Variant::Lazy));
                    assert_eq!(read, m.step(Variant::FullyLazy));
                    assert_eq!(read, m.step(Variant::Fused));
                    // Every upsert changed its key's value.
                    assert_eq!(m.delta.zset.len(), 2 * w.k);
                }
                assert_eq!(m.base.source, m.delta.source);
                assert_eq!(m.base.source, m.lazy.source);
                m.lazy.flush();
                assert_eq!(m.delta.filtered, m.lazy.filtered);
                m.fully.flush();
                assert_eq!(m.base.source, m.fully.source);
                assert_eq!(m.delta.filtered, m.fully.filtered);
                assert_eq!(m.base.source, m.fused.source);
                assert_eq!(m.delta.filtered, m.fused.filtered);
                assert_eq!(m.delta.count, m.fused.count);
            }
        }
    }

    /// The rope agrees with a `Vec` under random inserts and removes that
    /// grow it, shrink it to empty and grow it again, splitting and merging
    /// chunks on the way.
    #[test]
    fn rope_is_a_vec() {
        let mut rng = Rng::new(42);
        let start: Vec<u64> = (0..100).collect();
        let mut v = start.clone();
        let mut r = Rope::new(&start, 4);
        let (mut splits, mut merges) = (0, 0);
        for phase in [0.8, 0.2, 0.7] {
            for _ in 0..3_000 {
                let chunks = r.chunks.len();
                if v.is_empty() || (rng.below(1000) as f64) < phase * 1000.0 {
                    let i = rng.below(v.len() + 1);
                    let x = rng.next_u64();
                    v.insert(i, x);
                    r.insert(i, x);
                } else {
                    let i = rng.below(v.len());
                    assert_eq!(v.remove(i), r.remove(i));
                }
                splits += (r.chunks.len() > chunks) as usize;
                merges += (r.chunks.len() < chunks) as usize;
                assert_eq!(r.len(), v.len());
                assert!(r.chunks.iter().all(|c| c.len() <= 2 * r.b));
            }
            assert_eq!(r.to_vec(), v);
        }
        assert!(
            splits > 100 && merges > 100,
            "{splits} splits, {merges} merges"
        );
    }

    /// The B-tree agrees with a `Vec` under random inserts and removes that
    /// grow it past three levels, shrink it to empty and grow it again,
    /// and keeps its invariants throughout.
    #[test]
    fn btree_is_a_vec() {
        let mut rng = Rng::new(43);
        let start: Vec<u64> = (0..1_000).collect();
        let mut v = start.clone();
        let mut t = BTree::new(&start);
        t.check();
        let mut tallest = 0;
        for (phase, edits) in [(0.9, 60_000), (0.1, 80_000), (0.7, 5_000)] {
            for e in 0..edits {
                if v.is_empty() || (rng.below(1000) as f64) < phase * 1000.0 {
                    let i = rng.below(v.len() + 1);
                    let x = rng.next_u64();
                    v.insert(i, x);
                    t.insert(i, x);
                } else {
                    let i = rng.below(v.len());
                    assert_eq!(v.remove(i), t.remove(i));
                }
                if e % 997 == 0 {
                    t.check();
                    assert_eq!(t.len(), v.len());
                }
                tallest = tallest.max(t.height());
            }
            t.check();
            assert_eq!(t.to_vec(), v);
        }
        assert!(tallest >= 3, "height {tallest}");
        // Appends and removes at the end, the `append` workload's edits.
        for x in 0..10_000 {
            t.insert(t.len(), x);
            v.push(x);
        }
        for _ in 0..10_000 {
            assert_eq!(t.remove(t.len() - 1), v.pop().unwrap());
        }
        t.check();
        assert_eq!(t.to_vec(), v);
    }

    /// Later writes to one key win within the fully lazy buffer, and a
    /// write of the value already there changes nothing downstream.
    #[test]
    fn fully_lazy_last_write_wins() {
        let source: HashMap<u64, u64> = [(1, 1), (2, 2), (3, 3)].into_iter().collect();
        let mut m = FullyLazyMap {
            source: source.clone(),
            filtered: [(2, 2)].into_iter().collect(),
            count: 1,
            pending: Vec::new(),
            bound: FULLY_LAZY_BOUND,
        };
        m.commit(&[(1, 4), (2, 5)]);
        m.commit(&[(1, 7), (3, 3)]);
        m.commit(&[(2, 8), (1, 6)]);
        assert_eq!(m.source, source, "nothing applied before the read");
        assert_eq!(m.read(), 2);
        let expect: HashMap<u64, u64> = [(1, 6), (2, 8), (3, 3)].into_iter().collect();
        assert_eq!(m.source, expect);
        let expect: HashMap<u64, u64> = [(1, 6), (2, 8)].into_iter().collect();
        assert_eq!(m.filtered, expect);
    }

    /// The fully lazy design reads the same with a bound small enough to
    /// flush between reads.
    #[test]
    fn fully_lazy_bound() {
        for n in [3, 100] {
            for w in [ONE, TWO, RARE_READ] {
                let mut m = Maps::new(n, w);
                m.fully.bound = 3;
                for _ in 0..3 * CYCLE {
                    let read = m.step(Variant::Baseline);
                    assert_eq!(read, m.step(Variant::FullyLazy));
                    assert!(m.fully.pending.len() < m.fully.bound);
                }
            }
        }
    }

    /// The lazy design reads the same with a bound small enough to flush
    /// between reads, and never holds more than the bound after a commit.
    #[test]
    fn lazy_bound() {
        for n in [3, 100] {
            for w in [ONE, TWO, RARE_READ] {
                let mut m = Maps::new(n, w);
                m.lazy.bound = 6;
                for _ in 0..3 * CYCLE {
                    let read = m.step(Variant::Baseline);
                    assert_eq!(read, m.step(Variant::Lazy));
                    assert!(m.lazy.pending.len() < m.lazy.bound);
                }
            }
        }
    }

    /// Between reads, updates to one key cancel down to one retraction and
    /// one insertion: 15 instants on three keys leave 30 pending elements,
    /// which consolidate to at most six.
    #[test]
    fn lazy_cancels() {
        let mut m = Maps::new(3, RARE_READ);
        // The first instant reads, and the next 15 don't.
        m.steps(Variant::Lazy, RARE);
        assert_eq!(m.lazy.pending.len(), 2 * (RARE - 1));
        let mut z = m.lazy.pending.clone();
        consolidate(&mut z);
        assert!(z.len() <= 2 * 3, "{} elements", z.len());
    }

    /// Flo's eager-execution law for Z-sets: applying the composition of
    /// two deltas equals applying each in turn, when they touch distinct
    /// keys.
    #[test]
    fn composition_is_sequencing() {
        let mut m = Maps::new(100, TWO);
        let mut one_by_one = m.delta.clone();
        for t in 0..CYCLE {
            let upserts = m.trace[t].clone();
            m.delta.commit(&upserts);
            for u in &upserts {
                one_by_one.commit(std::slice::from_ref(u));
            }
            assert_eq!(m.delta.source, one_by_one.source);
            assert_eq!(m.delta.count, one_by_one.count);
        }
    }

    /// Two sources upserting one key in one instant, each against the
    /// state before the instant, compose into a Z-set that is not a map:
    /// the old value with weight -2 and two new values with weight +1. The
    /// group's `+` needs no combining function only for bags; a keyed
    /// collection needs one again, as `merge` does.
    #[test]
    fn same_key_conflict() {
        let integral: HashMap<u64, u64> = [(7, 1)].into_iter().collect();
        let mut z = Vec::new();
        compose_upserts(&integral, &[(7, 2), (7, 3)], &mut z);
        let weights: Vec<(u64, i64)> = z.iter().map(|e| (e.value, e.weight)).collect();
        assert_eq!(weights, vec![(1, -2), (2, 1), (3, 1)]);
    }
}
