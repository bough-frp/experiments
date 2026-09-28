//! A `'static` token in a `#[rebrand(skip)]` field. The committed derive
//! takes it: the field is cloned across brands as it is, and the `Trace`
//! written to agree with the skip (as a derived one would) doesn't trace it,
//! so the collector never sees it. The checked derive refuses it.
//!
//! A `'static` token comes only from the stash route of question 3, here
//! `stash-in-view.rs`'s: a hand-written `view` copies it out of the stored
//! value it is lent. The run then roots the stashed token two ways, directly
//! and inside a `Pinned`, drops every other root, and collects.
#![forbid(unsafe_code)]
use bough::*;

thread_local! {
    static STASH: std::cell::Cell<Option<Cell<'static, u32>>> = const { std::cell::Cell::new(None) };
}

struct Pair<'g> {
    left: Cell<'g, u32>,
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
        }
    }
    fn restore<'x>(from: &Pair<'x>, at: Witness<'x>) -> Self {
        Pair {
            left: Rebrand::restore(&from.left, at),
        }
    }
    fn view<'a>(from: &'a Pair<'static>) -> Option<&'a Self> {
        STASH.set(Some(from.left));
        None
    }
}

#[derive(Clone, Rebrand)]
struct Pinned {
    label: String,
    #[rebrand(skip)]
    count: Cell<'static, u32>,
}

/// What a derived `Trace` writes: the skipped field untraced.
impl Trace for Pinned {
    fn trace(&self, _: &mut Tracer) {}
}

/// Stash a token, root it directly or in a `Pinned`, drop the rest, collect,
/// and read it.
fn run(pinned: bool) -> Result<u32, Error> {
    let mut rt = Runtime::new();
    let (n_in, pair) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let left = n.hold(b, 0);
        let (never, _) = b.input::<Pair>();
        let pair = never.hold(b, Pair { left });
        (b.anchor(n_in), b.anchor(pair))
    });
    rt.send(&n_in, 3);
    let root = rt.mutate(|b| {
        // `map_cell` reads its input through `with_loaded`, which calls `view`.
        let pair = b.open(&pair);
        let _ = pair.map_cell(b, |p| p.left.index());
        let count = STASH.get().expect("stashed");
        if pinned {
            (
                None,
                Some(b.anchor(Pinned {
                    label: "pinned".to_string(),
                    count,
                })),
            )
        } else {
            (Some(b.anchor(count)), None)
        }
    });
    drop(pair);
    rt.collect();
    rt.mutate(|b| {
        // A `'static` token is read by anchoring it and opening the anchor.
        let anchored = match &root {
            (Some(direct), _) => direct.clone(),
            (_, Some(pinned)) => {
                let pinned = b.open(pinned);
                b.anchor(pinned.count)
            }
            _ => unreachable!(),
        };
        let count = b.open(&anchored);
        b.sample(count)
    })
}

fn main() {
    println!("stashed, anchored directly:    {:?}", run(false));
    println!("stashed, in a skipped field:   {:?}", run(true));
}
