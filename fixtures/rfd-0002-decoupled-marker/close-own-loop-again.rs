//! Illegal: F3 attempted with the returned token. The counter closes
//! legally; a second loop is then the F3 shape over the first's returned
//! token and its own forward, `d = hold (merge (steps c) (map (+1) (steps
//! d)))`. The returned token is fine to read; the second loop's own
//! forward is the cycle.
//@ legal no
//@ designs baseline marker-close
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-close.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let next_c = ticks.snapshot(c, |_, n| n + 1).hold(&mut b, 0);
    let c = c_loop.close(&mut b, next_c);

    let (d, d_loop) = b.cell_loop::<u32>();
    let next_d = c
        .steps()
        .merge(&mut b, d.steps().map(|n| n + 1))
        .hold(&mut b, 0);
    let _d = d_loop.close(&mut b, next_d);
}
