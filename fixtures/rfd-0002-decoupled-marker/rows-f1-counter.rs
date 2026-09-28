//! F1 under rows: the counter loop in slot `L0`, depending on no other loop.
//@ legal yes
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32, L0, Empty>();
    let next = ticks
        .snapshot(c, |_, n| n + 1)
        .filter(|n| *n <= 10)
        .hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
