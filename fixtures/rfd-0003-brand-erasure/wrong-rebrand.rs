//! Must build and run: question 1, what a wrong hand-written `Rebrand` can
//! do. Three wrong impls, each on the path a derive would write:
//!
//! - `Swapped` swaps two fields of the same type in `rebrand` only, so what
//!   a token names depends on how many rebrands its path took. `open`
//!   rebrands twice (at `anchor`, to `'static`, and at `open`) and the swaps
//!   cancel; a held initial value is stored once and restored once, so the
//!   sampled token names the other live node; after `map_to` feeds the
//!   cell, twice again.
//! - `Hidden` has a wrong `Trace` too, skipping a field its `Rebrand`
//!   keeps: that token is collected and its next use is stale.
//! - `Celsius` has an `Of` that doesn't round-trip (`Celsius` to
//!   `Kelvin` to `Rankine`): a value stored at one type is read at
//!   another, and the engine's downcast panics.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Clone, Copy)]
struct Swapped<'g> {
    left: Cell<'g, u32>,
    right: Cell<'g, u32>,
}

impl Trace for Swapped<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
        self.right.trace(tracer);
    }
}

impl<'g> Rebrand for Swapped<'g> {
    type Of<'x> = Swapped<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Swapped<'x> {
        Swapped {
            left: self.right.rebrand(to),
            right: self.left.rebrand(to),
        }
    }
    fn restore<'x>(from: &Swapped<'x>, at: Witness<'x>) -> Self {
        Swapped {
            left: Rebrand::restore(&from.left, at),
            right: Rebrand::restore(&from.right, at),
        }
    }
}

#[derive(Clone, Copy)]
struct Hidden<'g> {
    shown: Cell<'g, u32>,
    spare: Cell<'g, u32>,
}

/// Wrong: skips `spare`, as a `#[trace(skip)]` on it would.
impl Trace for Hidden<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.shown.trace(tracer);
    }
}

/// Wrong with it: hands `spare` back where `shown` was.
impl<'g> Rebrand for Hidden<'g> {
    type Of<'x> = Hidden<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Hidden<'x> {
        Hidden {
            shown: self.spare.rebrand(to),
            spare: self.spare.rebrand(to),
        }
    }
    fn restore<'x>(from: &Hidden<'x>, at: Witness<'x>) -> Self {
        Hidden {
            shown: Rebrand::restore(&from.shown, at),
            spare: Rebrand::restore(&from.spare, at),
        }
    }
}

#[derive(Clone)]
struct Celsius(u32);
#[derive(Clone)]
struct Kelvin(u32);
#[derive(Clone)]
struct Rankine(u32);

impl Rebrand for Celsius {
    type Of<'x> = Kelvin;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Kelvin {
        Kelvin(self.0 + 273)
    }
    fn restore<'x>(from: &Kelvin, _: Witness<'x>) -> Celsius {
        Celsius(from.0 - 273)
    }
}

impl Rebrand for Kelvin {
    type Of<'x> = Rankine;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Rankine {
        Rankine(self.0 * 9 / 5)
    }
    fn restore<'x>(from: &Rankine, _: Witness<'x>) -> Kelvin {
        Kelvin(from.0 * 5 / 9)
    }
}

impl Rebrand for Rankine {
    type Of<'x> = Rankine;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Rankine {
        self.clone()
    }
    fn restore<'x>(from: &Rankine, _: Witness<'x>) -> Rankine {
        from.clone()
    }
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, swapped, held, hidden) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let swapped = Swapped {
            left: n.map(|v| v + 1).hold(b, 1),
            right: n.map(|v| v + 1000).hold(b, 1000),
        };
        let hidden = Hidden {
            shown: n.map(|v| v + 2).hold(b, 2),
            spare: n.map(|v| v + 2000).hold(b, 2000),
        };
        println!(
            "swapped: built left at slot {}, right at slot {}",
            swapped.left.index(),
            swapped.right.index()
        );
        let held = n.map_to(swapped).hold(b, swapped);
        (
            b.anchor(n_in),
            b.anchor(swapped),
            b.anchor(held),
            b.anchor(hidden),
        )
    });
    let report = rt.mutate(|b| {
        let held = b.open(&held);
        let h = b.sample(held).expect("live");
        format!(
            "swapped: held initial value sampled, left at slot {} reads {:?}",
            h.left.index(),
            b.sample(h.left),
        )
    });
    println!("{report}");
    rt.send(&n_in, 5);
    let report = rt.mutate(|b| {
        let s = b.open(&swapped);
        let held = b.open(&held);
        let h = b.sample(held).expect("live");
        format!(
            "swapped: after a send, opened left at slot {} reads {:?}; held value sampled, left at slot {} reads {:?}",
            s.left.index(),
            b.sample(s.left),
            h.left.index(),
            b.sample(h.left),
        )
    });
    println!("{report}");
    drop((swapped, held));
    let report = rt.mutate(move |b| {
        b.collect();
        let h = b.open(&hidden);
        format!(
            "hidden:  after a collection, the reopened field reads {:?}",
            b.sample(h.shown)
        )
    });
    println!("{report}");

    let heat_in = rt.mutate(|b| {
        let (heat, heat_in) = b.input::<Celsius>();
        let _seen = heat.map(|c: Celsius| c.0).hold(b, 0);
        b.anchor(heat_in)
    });
    // The anchored input is `Input<'static, Kelvin>`, so the send takes a
    // `Kelvin`; opened, the input's event type is `Rankine`.
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        rt.mutate(|b| {
            let heat = b.open(&heat_in);
            b.send(heat, Rankine(540));
        })
    }));
    let outcome = match outcome {
        Ok(()) => "ran".to_string(),
        Err(panic) => format!(
            "panicked: {}",
            panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or(panic.downcast_ref::<&str>().copied())
                .unwrap_or("?")
        ),
    };
    println!("celsius: a send through the reopened input {outcome}");
}
