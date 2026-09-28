//! Must build: an anchored value holding tokens, which I/O code keeps
//! across units and listens to. The build's return is anchored like any
//! other (`b.anchor` inside a construct is in `screens.rs`).
#![feature(auto_traits, negative_impls)]
#[path = "api.rs"]
mod bough;
use bough::*;

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
    let built = rt.build(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let panels = Panels {
            left: n.hold(b, 0),
            right: n.map(|v| v * 2).hold(b, 0),
        };
        (n_in, panels)
    });
    rt.send(built.0, 3);
    let _heard = rt.listen_cell(built.1.right, |v| println!("{v}"));
}
