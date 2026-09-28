//! Illegal: a stream loop through `merge` only, `s = merge ticks s`.
//@ legal no
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (s, s_loop) = b.stream_loop::<u32>();
    let next = ticks.merge(&mut b, s);
    s_loop.close(&mut b, next);
}
