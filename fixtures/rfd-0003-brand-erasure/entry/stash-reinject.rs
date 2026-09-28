//! Builds, and shouldn't: the stash put back through a hand-written
//! `Rebrand`, which is handed a `Witness` for whatever brand it rebrands
//! to. `Sneak<'g>` is at one brand throughout, so every entry bound lets it
//! in; its `rebrand` swaps the stashed token in, at the brand `open` asks
//! for, and it reads live. No entry is involved: the witness is the route.
#![forbid(unsafe_code)]
#[path = "stash.rs"]
mod stash;
use bough::*;

thread_local! {
    static SNEAK: std::cell::Cell<Option<Cell<'static, u32>>> = const { std::cell::Cell::new(None) };
}

struct Sneak<'g> {
    cell: Cell<'g, u32>,
}

impl Trace for Sneak<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.cell.trace(tracer);
    }
}

impl<'g> Rebrand for Sneak<'g> {
    type Of<'x> = Sneak<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Sneak<'x> {
        let cell = match SNEAK.get() {
            // `Cell<'static, u32>::rebrand` is `Cell<'x, u32>`.
            Some(stashed) => stashed.rebrand(to),
            None => self.cell.rebrand(to),
        };
        Sneak { cell }
    }
    fn restore<'x>(from: &Sneak<'x>, at: Witness<'x>) -> Self {
        Sneak {
            cell: Rebrand::restore(&from.cell, at),
        }
    }
}

fn main() {
    let mut rt = Runtime::new();
    let (stashed, pair) = stash::stash(&mut rt);
    let sneak = rt.mutate(|b| {
        let (n, _) = b.input::<u32>();
        let cell = n.hold(b, 0);
        b.anchor(Sneak { cell })
    });
    SNEAK.set(Some(stashed));
    rt.mutate(|b| println!("opened, swapped in: {:?}", b.sample(b.open(&sneak).cell)));
    drop(pair);
    println!("collected:          {} nodes", rt.collect());
    rt.mutate(|b| println!("after:              {:?}", b.sample(b.open(&sneak).cell)));
}
