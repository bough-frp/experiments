//! Illegal: a split cuts only its own path. The definition merges the
//! split's output with the forward's steps directly, a same-instant cycle.
//@ legal no
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let (c, c_loop) = b.cell_loop::<u32>();
    let later = c.steps().map(|n| vec![n, n]).split(&mut b);
    let now = c.steps().map(|n| n + 1);
    let next = later.merge(&mut b, now).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
