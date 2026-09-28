//! Must build: an anchored value holding tokens, which I/O code keeps
//! across units and listens to by reopening it in a later `mutate`.
#[path = "api.rs"]
mod bough;
use bough::*;

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

impl Rebrand for Panels<'_> {
    type Of<'x> = Panels<'x>;
}

pub fn program() {
    let mut rt = Runtime::new();
    let (n_in, panels) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let panels = Panels {
            left: n.hold(b, 0),
            right: n.map(|v| v * 2).hold(b, 0),
        };
        (b.anchor(n_in), b.anchor(panels))
    });
    rt.send(&n_in, 3);
    let _heard = rt.mutate(|b| {
        let panels = b.open(&panels);
        b.listen_cell(panels.right, |v| println!("{v}"))
    });
}
