//! Must build and run: question 1. `Rebrand` written as a derive would
//! write it, for a struct of tokens with a `Vec` of them and for a generic
//! struct, crossing `mutate` through `anchor` and `open` with no `unsafe`
//! anywhere. The run prints the arena slots before and after, the values,
//! and what a collected or foreign token does.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

/// What `#[derive(Clone, Trace, Rebrand)]` would write.
#[derive(Clone)]
struct Panels<'g> {
    left: Cell<'g, u32>,
    right: Cell<'g, u32>,
    history: Vec<Cell<'g, u32>>,
}

impl Trace for Panels<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
        self.right.trace(tracer);
        self.history.trace(tracer);
    }
}

impl<'g> Rebrand for Panels<'g> {
    type Of<'x> = Panels<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Panels<'x> {
        Panels {
            left: self.left.rebrand(to),
            right: self.right.rebrand(to),
            history: self.history.rebrand(to),
        }
    }
    fn restore<'x>(from: &Panels<'x>, at: Witness<'x>) -> Self {
        Panels {
            left: Rebrand::restore(&from.left, at),
            right: Rebrand::restore(&from.right, at),
            history: Rebrand::restore(&from.history, at),
        }
    }
}

/// A generic one: the derive adds `T: Rebrand`, as it adds `T: Trace`.
#[derive(Clone)]
struct Tagged<T> {
    tag: String,
    value: T,
}

impl<T: Trace> Trace for Tagged<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.value.trace(tracer);
    }
}

impl<T: Rebrand> Rebrand for Tagged<T> {
    type Of<'x> = Tagged<T::Of<'x>>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        Tagged {
            tag: self.tag.rebrand(to),
            value: self.value.rebrand(to),
        }
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        Tagged {
            tag: Rebrand::restore(&from.tag, at),
            value: T::restore(&from.value, at),
        }
    }
}

fn slots(p: &Panels<'_>) -> String {
    let history: Vec<u32> = p.history.iter().map(|c| c.index()).collect();
    format!(
        "left {} right {} history {history:?}",
        p.left.index(),
        p.right.index()
    )
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, panels, total) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let left = n.hold(b, 0);
        let right = n.map(|v| v * 2).hold(b, 0);
        let history = vec![left, right, n.map(|v| v + 100).hold(b, 100)];
        let panels = Panels {
            left,
            right,
            history,
        };
        // A held value of tokens: the cell's value is itself a `Panels`.
        let held = n.map_to(panels.clone()).hold(b, panels.clone());
        let total = Tagged {
            tag: "total".to_string(),
            value: n.accumulate(b, 0u32, |v, sum| sum + v),
        };
        println!("built:   {}  total {}", slots(&panels), total.value.index());
        (b.anchor(n_in), b.anchor((panels, held)), b.anchor(total))
    });
    rt.send(&n_in, 3);
    rt.send(&n_in, 4);
    rt.mutate(|b| {
        let (panels, held) = b.open(&panels);
        let total = b.open(&total);
        println!("opened:  {}  total {}", slots(&panels), total.value.index());
        let from_cell = b.sample(held).expect("live");
        println!("held:    {}", slots(&from_cell));
        let history: Vec<_> = panels.history.iter().map(|c| b.sample(*c)).collect();
        println!(
            "values:  left {:?} right {:?} history {history:?} {} {:?}",
            b.sample(panels.left),
            b.sample(panels.right),
            total.tag,
            b.sample(total.value),
        );
    });
    let live = rt.live_nodes();
    // Drop every guard but the total's, collect, then use a token opened
    // before the drop: a stale token, found by its generation.
    drop(n_in);
    let stale = rt.mutate(move |b| {
        let (old, _) = b.open(&panels);
        drop(panels);
        let freed = b.collect();
        let total = b.open(&total);
        format!(
            "freed {freed} of {live} nodes; opened then collected: {:?}; still anchored: {:?}",
            b.sample(old.left),
            b.sample(total.value),
        )
    });
    println!("collect: {stale}");
    // An anchor opened in another runtime: the graph id catches it.
    let mut first = Runtime::new();
    let n_in = first.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let held = n.hold(b, 7);
        b.anchor((n_in, held))
    });
    let mut second = Runtime::new();
    let foreign = second.mutate(move |b| {
        let (_, held) = b.open(&n_in);
        b.sample(held)
    });
    println!("foreign: {foreign:?}");
}
