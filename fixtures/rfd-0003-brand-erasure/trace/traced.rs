//! Must build and run: `Trace` derived beside `Rebrand`, from the same
//! attributes, on a struct of tokens in every container, an enum and a
//! generic type, with a skipped field of a type Bough can't see into.
//!
//! Each token is a cell that nothing else roots: a hold of an input, which
//! the input doesn't keep (a node keeps what it depends on, not what depends
//! on it). A value is anchored, the graph collected, and every token in it
//! sampled: one the value's `Trace` missed was freed, and reads
//! `Err(Stale)`. The control is `Forgetful`, whose hand-written `Trace`
//! forgets a field, the bug a derived `Trace` can't have. Two roots are
//! tried: the value anchored, which traces it at `anchor`, and a cell
//! holding it anchored, which traces its stored copy at every collection.
#![forbid(unsafe_code)]
use bough::*;
use std::collections::HashMap;

/// Implements neither Bough trait, as another crate's type wouldn't.
#[derive(Clone, Debug)]
struct Celsius(f64);

#[derive(Clone, Rebrand, Trace)]
struct Panels<'g> {
    left: Cell<'g, u32>,
    history: Vec<Cell<'g, u32>>,
    pinned: Option<Cell<'g, u32>>,
    boxed: Box<Cell<'g, u32>>,
    named: HashMap<String, Cell<'g, u32>>,
    screen: Screen<'g>,
    tagged: Tagged<Cell<'g, u32>>,
    #[rebrand(skip)]
    outside: Celsius,
}

#[derive(Clone, Copy, Rebrand, Trace)]
enum Screen<'g> {
    Login { user: Cell<'g, u32> },
    Chat(Cell<'g, u32>, Cell<'g, u32>),
    Closed,
}

#[derive(Clone, Rebrand, Trace)]
struct Tagged<T> {
    tag: String,
    value: T,
}

#[derive(Clone, Rebrand)]
struct Forgetful<'g> {
    left: Cell<'g, u32>,
    pinned: Option<Cell<'g, u32>>,
}

/// By hand, with `pinned` forgotten.
impl Trace for Forgetful<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
    }
}

fn panel_tokens<'g>(p: &Panels<'g>) -> Vec<Cell<'g, u32>> {
    let mut all = vec![p.left];
    all.extend(&p.history);
    all.extend(p.pinned);
    all.push(*p.boxed);
    all.extend(p.named.values());
    match p.screen {
        Screen::Login { user } => all.push(user),
        Screen::Chat(a, b) => all.extend([a, b]),
        Screen::Closed => {}
    }
    all.push(p.tagged.value);
    all
}

fn forgetful_tokens<'g>(f: &Forgetful<'g>) -> Vec<Cell<'g, u32>> {
    let mut all = vec![f.left];
    all.extend(f.pinned);
    all
}

/// `count` cells, each a hold of the input `n` that nothing roots.
fn cells<'g>(b: &mut Build<'g>, n: Shared<'g, u32>, count: u32) -> Vec<Cell<'g, u32>> {
    (0..count)
        .map(|k| n.map(move |v| v + 100 * k).hold(b, 0))
        .collect()
}

fn panels<'g>(c: &[Cell<'g, u32>]) -> Panels<'g> {
    Panels {
        left: c[0],
        history: vec![c[1], c[2]],
        pinned: Some(c[3]),
        boxed: Box::new(c[4]),
        named: HashMap::from([("five".to_string(), c[5])]),
        screen: Screen::Chat(c[6], c[7]),
        tagged: Tagged {
            tag: "eight".to_string(),
            value: c[8],
        },
        outside: Celsius(21.5),
    }
}

/// A cell holding `value`: an input never sent, held.
fn held<'g, T: Rebrand + Trace>(b: &mut Build<'g>, value: T) -> Cell<'g, T> {
    let (never, _) = b.input::<T>();
    never.hold(b, value)
}

fn report<'g>(b: &Build<'g>, name: &str, freed: usize, tokens: &[Cell<'g, u32>]) {
    let reads: Vec<_> = tokens.iter().map(|c| b.sample(*c)).collect();
    let live = reads.iter().filter(|r| r.is_ok()).count();
    println!(
        "{name:<34} {live} of {} tokens live, {freed} nodes freed",
        tokens.len()
    );
    if live < tokens.len() {
        println!("{:<34} {reads:?}", "");
    }
}

fn main() {
    // The value anchored: traced once, at `anchor`.
    let mut rt = Runtime::new();
    let (n_in, p, f) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let c = cells(b, n, 11);
        let f = Forgetful {
            left: c[9],
            pinned: Some(c[10]),
        };
        (b.anchor(n_in), b.anchor(panels(&c)), b.anchor(f))
    });
    rt.send(&n_in, 1);
    let freed = rt.collect();
    rt.mutate(|b| {
        let p = b.open(&p);
        println!("skipped field:                     {:?}", p.outside);
        report(b, "Panels, derived, anchored:", freed, &panel_tokens(&p));
        let f = b.open(&f);
        report(
            b,
            "Forgetful, by hand, anchored:",
            freed,
            &forgetful_tokens(&f),
        );
    });

    // A cell holding the value anchored: traced from its stored copy.
    let mut rt = Runtime::new();
    let (n_in, p, f) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let c = cells(b, n, 11);
        let f = Forgetful {
            left: c[9],
            pinned: Some(c[10]),
        };
        let p = held(b, panels(&c));
        let f = held(b, f);
        (b.anchor(n_in), b.anchor(p), b.anchor(f))
    });
    rt.send(&n_in, 1);
    let freed = rt.collect();
    rt.mutate(|b| {
        let p = b.sample(b.open(&p)).expect("the cell is anchored");
        report(b, "Panels, derived, in a cell:", freed, &panel_tokens(&p));
        let f = b.sample(b.open(&f)).expect("the cell is anchored");
        report(
            b,
            "Forgetful, by hand, in a cell:",
            freed,
            &forgetful_tokens(&f),
        );
    });
}
