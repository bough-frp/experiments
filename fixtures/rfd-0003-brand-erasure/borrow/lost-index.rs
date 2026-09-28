//! Must fail, recording what the read view gives up: `list[i]`. A
//! `ListRef` has `get(i)`, returning an element's view by value, and no
//! `Index`, which must return a reference to an element at the reader's
//! brand that the stored copy doesn't contain.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Panels<'g> {
    left: Cell<'g, u32>,
    history: Vec<Cell<'g, u32>>,
}

impl Trace for Panels<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
        self.history.trace(tracer);
    }
}

fn build<'g>(b: &mut Build<'g>) -> Cell<'g, Panels<'g>> {
    let (n, _) = b.input::<u32>();
    let left = n.hold(b, 0);
    let (never, _) = b.input::<Panels>();
    never.hold(
        b,
        Panels {
            left,
            history: vec![left],
        },
    )
}

fn main() {
    let mut rt = Runtime::new();
    rt.mutate(|b| {
        let panels = build(b);
        let view = b.sample_ref(panels).expect("live");
        let first = view.history[0];
        let _ = b.sample(first);
    });
}
