//! Must build: `hold` of a struct holding tokens. The struct carries the
//! brand as a lifetime parameter.
#[path = "api.rs"]
mod bough;
use bough::*;

/// What `#[derive(Clone, Copy, Trace)]` gives, with the brand.
#[derive(Clone, Copy)]
struct Panels<'g> {
    left: Cell<'g, u32>,
    right: Cell<'g, u32>,
}

impl Trace for Panels<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
        self.right.trace(tracer);
    }
}

/// And what the derive would add for anything that crosses `mutate`.
impl Rebrand for Panels<'_> {
    type Of<'x> = Panels<'x>;
}

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let first = Panels {
            left: n.hold(b, 0),
            right: n.map(|v| v + 1).hold(b, 1),
        };
        let panels = n.map_to(first).hold(b, first);
        b.anchor((n_in, panels))
    });
}
