//! Illegal: a stream loop and a cell loop, the stream closed from the
//! cell's steps while the cell is open, the cell then closed from the
//! stream's returned token. `s = map (+1) (steps c)`, `c = hold 0 (merge
//! ticks s)`: F3 with a stream loop between. Closing the stream first is
//! the only order that could use a returned token, and the stream's own
//! definition reads the open cell.
//@ legal no
//@ designs baseline marker-close
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-close.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let (s, s_loop) = b.stream_loop::<u32>();
    let _ = s;
    let s = s_loop.close(&mut b, c.steps().map(|n| n + 1));
    let next_c = ticks.merge(&mut b, s).hold(&mut b, 0);
    let _c = c_loop.close(&mut b, next_c);
}
