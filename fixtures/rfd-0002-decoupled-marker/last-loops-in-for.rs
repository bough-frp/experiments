//! A counter per name, in a `for` loop over names known at build. Legal,
//! but each close moves the epoch on, so the build's type changes each
//! time round.
//@ legal yes
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program(names: &[&str]) {
    let mut b = Build::new();
    for _name in names {
        let ticks = b.input::<()>();
        let (mut looped, c, c_loop) = b.cell_loop::<u32>();
        let next = ticks.snapshot(c, |_, n| n + 1).hold(&mut looped, 0);
        let (closed, _c) = c_loop.close(looped, next);
        b = closed;
    }
    let _ = b;
}
