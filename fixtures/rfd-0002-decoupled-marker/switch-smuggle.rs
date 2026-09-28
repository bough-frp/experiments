//! Illegal: a switch whose inner is the switch's own consumer, reached
//! through tokens as data. `inners` is a loop over stream tokens, typed
//! `Decoupled`; its forward is `Instantaneous`, but `switch_stream` reads
//! its selection from before the instant and drops that. So `total` is
//! `Decoupled`, and `inners` closes with a constant holding `total`'s own
//! steps: the switch's output depends on `total`, and `total` on it, in
//! the same instant. The engine refuses it at the switch's first link.
//@ legal no
//@ designs baseline marker marker-close
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (inners, inners_loop) = b.cell_loop::<Stream<u32>>();
    let from_inner = inners.switch_stream(&mut b);
    let total = ticks.merge(&mut b, from_inner).hold(&mut b, 0);
    let inner = b.constant(total.steps());
    inners_loop.close(&mut b, inner);
}
