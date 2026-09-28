//! Must fail, recording what the read view gives up: the slice. A
//! `ListRef` of tokens has no `&[Cell<'g, u32>]` to give; `as_slice` exists
//! only where the element is its own stored copy, and a token's stored copy
//! is `Cell<'static, u32>`.
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

fn total<'g>(b: &Build<'g>, xs: &[Cell<'g, u32>]) -> u32 {
    xs.iter().map(|c| b.sample(*c).unwrap_or(0)).sum()
}

fn main() {
    let mut rt = Runtime::new();
    rt.mutate(|b| {
        let panels = build(b);
        let view = b.sample_ref(panels).expect("live");
        let _ = total(b, view.history.as_slice());
    });
}
