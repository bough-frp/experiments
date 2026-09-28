//! Must fail: a view claimed at brand `'static`, the brand of the stored copy
//! it was built from, to get a `'static` token out of it.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Pair<'g> {
    left: Cell<'g, u32>,
    label: String,
}

impl Trace for Pair<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
    }
}

/// An input, and a cell holding a `Pair` over it.
fn build<'g>(b: &mut Build<'g>) -> (Input<'g, u32>, Cell<'g, Pair<'g>>) {
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
    (n_in, pair)
}

thread_local! {
    static STASH: std::cell::Cell<Option<Cell<'static, u32>>> = const { std::cell::Cell::new(None) };
}

fn main() {
    let mut rt = Runtime::new();
    rt.mutate(|b| {
        let (_, pair) = build(b);
        let view: PairRef<'_, 'static> = b.sample_ref(pair).expect("live");
        STASH.set(Some(view.left));
    });
}
