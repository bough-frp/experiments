//! Illegal: `switch-smuggle` in the rows design. The loop over stream
//! tokens declares no dependencies; the switch drops the outer's row, so
//! `total` has the empty row and the constant closes the loop.
//@ legal no
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (inners, inners_loop) = b.cell_loop::<Stream<u32>, L0, Empty>();
    let from_inner = inners.switch_stream(&mut b);
    let total = ticks.merge(&mut b, from_inner).hold(&mut b, 0);
    let inner = b.constant(total.steps());
    inners_loop.close(&mut b, inner);
}
