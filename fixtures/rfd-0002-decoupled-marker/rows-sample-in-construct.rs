//! `sample-in-construct` under rows.
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
        .construct(&mut b, move |b, _| *c.sample(b) + 1)
        .hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
