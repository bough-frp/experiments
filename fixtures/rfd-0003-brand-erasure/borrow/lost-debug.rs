//! Must fail, recording what the read view gives up: the traits the value
//! implements. `Panels` is `Debug`; its generated `PanelsRef` isn't, and a
//! derive or impl on the original doesn't reach it.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Panels<'g> {
    left: Cell<'g, u32>,
    history: Vec<Cell<'g, u32>>,
}

/// By hand: tokens aren't `Debug`.
impl std::fmt::Debug for Panels<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Panels(#{}, {} in history)",
            self.left.index(),
            self.history.len()
        )
    }
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
        println!("{view:?}");
    });
}
