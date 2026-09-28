//! Must fail: a view held across a `send`. RFD 4 makes holding `&A` from
//! `sample` across a `send` a borrow error (E0502); a view borrows `b` the
//! same way.
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
    rt.mutate(|b| {
        let (n_in, pair) = build(b);
        let view = b.sample_ref(pair).expect("live");
        b.send(n_in, 1);
        println!("{}", view.label);
    });
}
