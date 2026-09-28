//! Illegal: a stream loop through `map` only, `s = map (+1) s`.
//@ legal no
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let (s, s_loop) = b.stream_loop::<u32>();
    s_loop.close(&mut b, s.map(|n| n + 1));
}
