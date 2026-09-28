//! Builds, and shouldn't: a hand-written `Borrow` impl copies a token out of
//! the stored copy it is lent, where it is `Cell<'static, u32>`, into a
//! thread-local. `borrow` must be handed the stored `Of<'static>`, which is
//! what makes the view free, and a token in it is `Copy`, so any impl can keep
//! one: the forgotten capture the brand exists to stop, through the impl
//! rather than a closure. The derive never writes this; nothing stops a hand.
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
}

struct PairRef<'a, 'g> {
    left: Cell<'g, u32>,
    label: &'a String,
}

impl<'g> Borrow for Pair<'g> {
    type Ref<'a> = PairRef<'a, 'g>;
    fn borrow<'a>(from: &'a Pair<'static>, at: Loan<'a>) -> PairRef<'a, 'g> {
        STASH.set(Some(from.left));
        PairRef {
            left: <Cell<'g, u32> as Borrow>::borrow(&from.left, at),
            label: &from.label,
        }
    }
}

#[allow(non_snake_case)]
fn READ<'g>(b: &Build<'g>, pair: Cell<'g, Pair<'g>>) {
    let view = b.sample_ref(pair).expect("live");
    println!(
        "sampled:             {} {:?}",
        view.label,
        b.sample(view.left)
    );
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
