//! Must build and run: RFD 4's read-through `map_cell`, whose memo is a
//! `OnceCell` that `sample` returns `&A` from. Here the memo holds the
//! value's stored copy, `B::Of<'static>`, and `sample_ref` returns
//! `B::Ref<'_>` built from it, with no `unsafe`. The memoized value holds a
//! token, picked by the function from a struct of tokens. The run counts the
//! function's calls: one per step that is read, none for a step that isn't.
#![forbid(unsafe_code)]
use bough::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Rebrand)]
struct Pair<'g> {
    even: Cell<'g, u32>,
    odd: Cell<'g, u32>,
    n: u32,
}

#[derive(Rebrand)]
struct Tagged<T> {
    tag: String,
    value: T,
}

impl Trace for Pair<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.even.trace(tracer);
        self.odd.trace(tracer);
    }
}

impl<T: Trace> Trace for Tagged<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.value.trace(tracer);
    }
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, picked) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let even = n.map(|v| v * 2).hold(b, 0);
        let odd = n.map(|v| v * 2 + 1).hold(b, 1);
        let pair = n
            .with((even, odd))
            .map(|((even, odd), n)| Pair { even, odd, n })
            .hold(b, Pair { even, odd, n: 0 });
        let picked = pair.map_cell_lazy(b, |p| {
            CALLS.fetch_add(1, Ordering::Relaxed);
            Tagged {
                tag: format!("n={}", p.n),
                value: if p.n % 2 == 0 { p.even } else { p.odd },
            }
        });
        (b.anchor(n_in), b.anchor(picked))
    });
    let read = |rt: &mut Runtime, label: &str| {
        rt.mutate(|b| {
            let picked = b.open(&picked);
            let first = b.sample_ref(picked).expect("live");
            let second = b.sample_ref(picked).expect("live");
            println!(
                "{label:<8} {} -> {:?}, again {} -> {:?}, calls so far {}",
                first.tag,
                b.sample(first.value),
                second.tag,
                b.sample(second.value),
                CALLS.load(Ordering::Relaxed),
            );
        })
    };
    read(&mut rt, "initial");
    rt.send(&n_in, 3);
    read(&mut rt, "after 3");
    // Two steps with no read between: the function runs for the last only.
    rt.send(&n_in, 4);
    rt.send(&n_in, 6);
    read(&mut rt, "after 6");
}
