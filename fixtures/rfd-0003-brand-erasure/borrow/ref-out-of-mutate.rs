//! Must fail: a view returned from the `mutate` that sampled it, to be read
//! after the call.
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

fn main() {
    let mut rt = Runtime::new();
    let view = rt.mutate(|b| {
        let (_, pair) = build(b);
        b.sample_ref(pair).expect("live")
    });
    let _ = view;
}
