//! Must build: `hold` of a struct holding tokens, traced through its derive.
#[path = "api.rs"]
mod bough;
use bough::*;

/// What `#[derive(Clone, Copy, Trace)]` gives.
#[derive(Clone, Copy)]
struct Panels {
    left: Cell<u32>,
    right: Cell<u32>,
}

impl Trace for Panels {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
        self.right.trace(tracer);
    }
}

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let first = Panels {
            left: n.hold(b, 0),
            right: n.map(|v| v + 1).hold(b, 1),
        };
        let panels = n.map_to(first).hold(b, first);
        (n_in, panels)
    });
}
