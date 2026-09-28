//! `gate-counter` under rows.
//@ legal yes
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32, L0, Empty>();
    let below_ten = c.map_cell(&mut b, |n| *n < 10);
    let next = ticks
        .gate(below_ten)
        .snapshot(c, |_, n| n + 1)
        .hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
