//! Illegal: `close-switch-smuggle` in the marker-last design. The
//! counter's close leaves the inners' loop open; the returned token is
//! `Decoupled` anyway, and the switch's output carries the inner's mark.
//@ legal no
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (b, c, c_loop) = b.cell_loop::<u32>();
    let (mut b, inners, inners_loop) = b.cell_loop::<Stream<u32>>();
    let from_inner = inners.switch_stream(&mut b);
    let next_c = ticks
        .merge(&mut b, from_inner)
        .snapshot(c, |d, n| n + d)
        .hold(&mut b, 0);
    let (mut b, c) = c_loop.close(b, next_c);
    let inner = b.constant(c.steps());
    let (_b, _inners) = inners_loop.close(b, inner);
}
