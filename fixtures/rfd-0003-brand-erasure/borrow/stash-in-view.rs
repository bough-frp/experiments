//! Builds, and shouldn't: the same stash through the committed `Rebrand::view`,
//! which is also handed the stored `&Self::Of<'static>`. The main run's
//! `api.rs` has this route already; `borrow/api.rs` is that file, unchanged
//! above its views, and this fixture uses nothing below them.
#![forbid(unsafe_code)]
use bough::*;

thread_local! {
    static STASH: std::cell::Cell<Option<Cell<'static, u32>>> = const { std::cell::Cell::new(None) };
}

struct Pair<'g> {
    left: Cell<'g, u32>,
    label: String,
}

impl Trace for Pair<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
    }
}

impl<'g> Rebrand for Pair<'g> {
    type Of<'x> = Pair<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Pair<'x> {
        Pair {
            left: self.left.rebrand(to),
            label: self.label.clone(),
        }
    }
    fn restore<'x>(from: &Pair<'x>, at: Witness<'x>) -> Self {
        Pair {
            left: Rebrand::restore(&from.left, at),
            label: from.label.clone(),
        }
    }
    fn view<'a>(from: &'a Pair<'static>) -> Option<&'a Self> {
        STASH.set(Some(from.left));
        None
    }
}

#[allow(non_snake_case)]
fn READ<'g>(b: &mut Build<'g>, pair: Cell<'g, Pair<'g>>) {
    // `map_cell` reads its input through `with_loaded`, which calls `view`.
    let len = pair.map_cell(b, |p| p.label.len() as u32);
    println!("mapped:              {:?}", b.sample(len));
}

/// Mutate 1 builds a hold and a cell holding a `Pair` over it, anchored.
/// Mutate 2 reads the cell, which runs the impl that stashes the token.
/// Then the anchor is dropped and the graph collected, and mutate 3 opens
/// the stashed token: nothing rooted it, so it is stale.
fn main() {
    let mut rt = Runtime::new();
    let (n_in, pair) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let left = n.hold(b, 0);
        let (never, _) = b.input::<Pair>();
        let pair = never.hold(
            b,
            Pair {
                left,
                label: "pair".to_string(),
            },
        );
        (b.anchor(n_in), b.anchor(pair))
    });
    rt.send(&n_in, 3);
    rt.mutate(|b| {
        let pair = b.open(&pair);
        READ(b, pair);
        let anchored = b.anchor(STASH.get().expect("stashed"));
        let stashed = b.open(&anchored);
        println!("stashed, anchored:   {:?}", b.sample(stashed));
    });
    drop(pair);
    println!("collected:           {} nodes", rt.collect());
    rt.mutate(|b| {
        let anchored = b.anchor(STASH.get().expect("stashed"));
        let stashed = b.open(&anchored);
        println!("stashed, after:      {:?}", b.sample(stashed));
    });
}
