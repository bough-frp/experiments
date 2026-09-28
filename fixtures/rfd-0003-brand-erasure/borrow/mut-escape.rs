//! Must fail: a write view kept past the `update` that lent it, to write the
//! stored copy later, outside any unit.
#![forbid(unsafe_code)]
use bough::*;

thread_local! {
    static STASH: std::cell::Cell<Option<Cell<'static, u32>>> = const { std::cell::Cell::new(None) };
}

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

/// A cell holding a `Panels` over a hold.
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
        let mut kept = None;
        b.update(panels, |p| kept = Some(p.into_mut().history))
            .expect("live");
        if let Some(mut list) = kept {
            list.clear();
        }
    });
}
