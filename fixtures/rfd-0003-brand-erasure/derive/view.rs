//! Must build and run: the copy-free `view`. The derive writes
//! `view = Some(from)` for a type with no lifetime and no type parameter,
//! whose stored copy is itself; the engine's reads (`listen_cell`,
//! `snapshot`, `map_cell`, `accumulate`) then borrow it in place. Any other
//! type is restored, a copy per read.
//!
//! `Counted` is a brand-free leaf, hand-written, that counts its restores.
//! The same leaf sits in three derived types, each held in a cell and read
//! by a listener on every event:
//!
//! - `Stats`: no brand, no parameter, so it has a view.
//! - `Branded<'g>`: a brand, so no view, though this run holds no token.
//! - `Tagged<Counted>`: a parameter, so no view, though it is brand-free.
#![forbid(unsafe_code)]
use bough::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static RESTORES: AtomicUsize = AtomicUsize::new(0);

/// Not `Clone`: a derived `view` doesn't need it.
struct Counted(u32);

impl Rebrand for Counted {
    type Of<'x> = Counted;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Counted {
        Counted(self.0)
    }
    fn restore<'x>(from: &Counted, _: Witness<'x>) -> Counted {
        RESTORES.fetch_add(1, Ordering::Relaxed);
        Counted(from.0)
    }
    fn view(from: &Counted) -> Option<&Counted> {
        Some(from)
    }
}

impl Trace for Counted {
    fn trace(&self, _: &mut Tracer) {}
}

#[derive(Rebrand)]
struct Stats {
    events: u32,
    last: Counted,
}

#[derive(Rebrand)]
struct Branded<'g> {
    last: Counted,
    source: Option<Cell<'g, u32>>,
}

#[derive(Rebrand)]
struct Tagged<T> {
    tag: String,
    value: T,
}

impl Trace for Stats {
    fn trace(&self, _: &mut Tracer) {}
}

impl Trace for Branded<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.source.trace(tracer);
    }
}

impl<T: Trace> Trace for Tagged<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.value.trace(tracer);
    }
}

const EVENTS: u32 = 5;

/// Hold `make`'s value of each event in a cell, read it in a listener on
/// every event, and count the restores the reads cost.
fn reads<T: Rebrand + Trace + 'static>(
    name: &str,
    init: fn(u32) -> T,
    make: fn(u32) -> T,
    last: fn(&T) -> u32,
) {
    let mut rt = Runtime::new();
    let n_in = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let held = n.map(make).hold(b, init(0));
        b.listen_cell(held, move |t| assert!(last(t) > 0)).keep();
        b.anchor(n_in)
    });
    RESTORES.store(0, Ordering::Relaxed);
    for i in 1..=EVENTS {
        rt.send(&n_in, i);
    }
    let restores = RESTORES.load(Ordering::Relaxed);
    let view = if restores == 0 { "borrowed" } else { "copied" };
    println!("{name:<16} {EVENTS} reads, {restores} restores: {view}");
}

fn main() {
    reads(
        "Stats",
        |i| Stats {
            events: i,
            last: Counted(i),
        },
        |i| Stats {
            events: i,
            last: Counted(i),
        },
        |s| s.last.0 + s.events,
    );
    reads(
        "Branded<'g>",
        |i| Branded {
            last: Counted(i),
            source: None,
        },
        |i| Branded {
            last: Counted(i),
            source: None,
        },
        |s| s.last.0,
    );
    reads(
        "Tagged<Counted>",
        |i| Tagged {
            tag: "last".to_string(),
            value: Counted(i),
        },
        |i| Tagged {
            tag: "last".to_string(),
            value: Counted(i),
        },
        |t| t.value.0,
    );
}
