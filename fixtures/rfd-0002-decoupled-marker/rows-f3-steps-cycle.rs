//! F3 under rows.
//@ legal no
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (c, c_loop) = b.cell_loop::<u32, L0, Empty>();
    let bumped = c.steps().map(|n| n + 1);
    let next = ticks.merge(&mut b, bumped).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
