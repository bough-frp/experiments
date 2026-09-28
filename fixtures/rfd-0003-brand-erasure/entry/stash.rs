//! The stash the `entry` fixtures share: `borrow/stash-in-view.rs`'s route,
//! a hand-written `Rebrand::view` that copies the `'static` token out of the
//! stored copy it is lent. Not through `Borrow`, so that it still builds
//! against the sealed views of question 3, and the fixtures that use it ask
//! only what a stashed token can do once it is out.
use bough::*;

thread_local! {
    static STASH: std::cell::Cell<Option<Cell<'static, u32>>> = const { std::cell::Cell::new(None) };
}

pub struct Pair<'g> {
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

/// Mutate 1 builds a hold of 3 and a cell holding a `Pair` over it, both
/// rooted by the returned anchor; mutate 2 reads the pair through
/// `map_cell`, which calls `view`, which stashes the hold's token. Returns
/// the stashed token and the anchor; dropping the anchor unroots the hold.
pub fn stash(rt: &mut Runtime) -> (Cell<'static, u32>, Anchored<Cell<'static, Pair<'static>>>) {
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
        let _ = pair.map_cell(b, |p| p.label.len() as u32);
    });
    (STASH.get().expect("stashed"), pair)
}
