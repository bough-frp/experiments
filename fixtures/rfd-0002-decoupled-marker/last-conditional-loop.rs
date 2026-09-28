//! A counter built only when `debug` is set. Legal, but the two branches
//! leave the build in different epochs, so their types differ.
//@ legal yes
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program(debug: bool) {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let b = if debug {
        let (mut b, c, c_loop) = b.cell_loop::<u32>();
        let next = ticks.snapshot(c, |_, n| n + 1).hold(&mut b, 0);
        let (b, _c) = c_loop.close(b, next);
        b
    } else {
        b
    };
    let _ = b;
}
