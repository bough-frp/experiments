//! `sample-in-construct` with the reach the construct's documentation asks
//! for: the closure captures the forward, which its stream doesn't depend
//! on, so `depends` declares it. A reach, not a dependency, so still legal.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let made = ticks.construct(&mut b, move |b, _| *c.sample(b) + 1);
    b.depends(&made, &[&c]);
    let next = made.hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
