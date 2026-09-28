//! Must build and run: copy-free reads of a generic type instantiated
//! brand-free, around `view`. `derive/view.rs` found `Tagged<Counted>`
//! restored on every read though it holds no token, since the derive can't
//! write `view` for a generic type. Here the same cell is read three ways,
//! five events each, counting the restores the reads cost:
//!
//! - `listen_cell`, through `view`: a copy per read, as before.
//! - `listen_cell_plain`, bounded by `A: Rebrand<Of<'static> = A>`: the
//!   bound says the stored copy is `A`, so it is lent as it is.
//! - `listen_cell_static`, bounded by `A: 'static`: the stored copy
//!   downcast to `A` at run time, a restore if that fails.
//!
//! `Stats`, which has no parameter, is the control: `view` covers it.
#![forbid(unsafe_code)]
use bough::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static RESTORES: AtomicUsize = AtomicUsize::new(0);

/// A brand-free leaf, by hand, that counts its restores.
struct Counted(u32);

impl Trace for Counted {
    fn trace(&self, _: &mut Tracer) {}
}

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

#[derive(Rebrand, Trace)]
struct Tagged<T> {
    tag: String,
    value: T,
}

#[derive(Rebrand, Trace)]
struct Stats {
    events: u32,
    last: Counted,
}

const EVENTS: u32 = 5;

#[derive(Clone, Copy)]
enum Read {
    View,
    Plain,
    Static,
}

/// Hold `make`'s value of each event in a cell, read it in a listener on
/// every event, and count the restores.
fn reads<T: Rebrand<Of<'static> = T> + Trace + 'static>(
    name: &str,
    read: Read,
    make: fn(u32) -> T,
    last: fn(&T) -> u32,
) {
    let mut rt = Runtime::new();
    let n_in = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let held = n.map(make).hold(b, make(0));
        let check = move |t: &T| assert!(last(t) > 0);
        match read {
            Read::View => b.listen_cell(held, check),
            Read::Plain => b.listen_cell_plain(held, check),
            Read::Static => b.listen_cell_static(held, check),
        }
        .keep();
        b.anchor(n_in)
    });
    RESTORES.store(0, Ordering::Relaxed);
    for i in 1..=EVENTS {
        rt.send(&n_in, i);
    }
    let restores = RESTORES.load(Ordering::Relaxed);
    let how = match read {
        Read::View => "listen_cell",
        Read::Plain => "listen_cell_plain",
        Read::Static => "listen_cell_static",
    };
    let view = if restores == 0 { "borrowed" } else { "copied" };
    println!("{name:<24} {how:<19} {EVENTS} reads, {restores} restores: {view}");
}

fn tagged(i: u32) -> Tagged<Counted> {
    Tagged {
        tag: "last".to_string(),
        value: Counted(i),
    }
}

fn nested(i: u32) -> Tagged<Tagged<Counted>> {
    Tagged {
        tag: "outer".to_string(),
        value: tagged(i),
    }
}

fn stats(i: u32) -> Stats {
    Stats {
        events: i,
        last: Counted(i),
    }
}

fn main() {
    for read in [Read::View, Read::Plain, Read::Static] {
        reads("Stats", read, stats, |s| s.last.0 + s.events);
        reads("Tagged<Counted>", read, tagged, |t| t.value.0);
        reads("Tagged<Tagged<Counted>>", read, nested, |t| t.value.value.0);
    }
}
