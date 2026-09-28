//! Must fail, recording what the write view gives up: the `Vec` and slice
//! methods `ListMut` doesn't wrap, here `dedup_by_key`. Each needs a wrapper
//! that shows its closure views, as `retain` and `sort_by` have; the ones
//! that don't take a closure (`drain`, `split_off`, `extend_from_slice`)
//! would move stored elements and need a restore each.
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
        b.update(panels, |mut p| {
            p.as_mut().history.dedup_by_key(|c| c.index())
        })
        .expect("live");
    });
}
