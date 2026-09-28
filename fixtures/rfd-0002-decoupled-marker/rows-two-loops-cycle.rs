//! `two-loops-cycle` under rows, with each declaration listing the other
//! loop, the most a user could declare.
//@ legal no
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (x, x_loop) = b.cell_loop::<u32, L0, Only<L1>>();
    let (y, y_loop) = b.cell_loop::<u32, L1, Only<L0>>();
    let next_x = ticks
        .merge(&mut b, y.steps().map(|n| n + 1))
        .hold(&mut b, 0);
    x_loop.close(&mut b, next_x);
    let next_y = x.steps().hold(&mut b, 0);
    y_loop.close(&mut b, next_y);
}
