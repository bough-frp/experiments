//! Illegal: `close-stream-loop-cycle` in the marker-last design. The
//! stream closes first, from the open cell's steps.
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
    let (b, s, s_loop) = b.stream_loop::<u32>();
    let _ = s;
    let (mut b, s) = s_loop.close(b, c.steps().map(|n| n + 1));
    let next_c = ticks.merge(&mut b, s).hold(&mut b, 0);
    let (_b, _c) = c_loop.close(b, next_c);
}
