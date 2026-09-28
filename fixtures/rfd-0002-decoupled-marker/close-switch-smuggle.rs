//! Illegal: the returned token read into a cycle. A counter's definition
//! merges a switch over a loop of stream tokens; after the counter closes,
//! its returned token's steps become the switch's inner. The re-marked
//! token is honest about what it reaches at the close; the switch's
//! output is what grows afterwards. `switch-smuggle` does the same with
//! the definition token in the marker design.
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
    let (inners, inners_loop) = b.cell_loop::<Stream<u32>>();
    let from_inner = inners.switch_stream(&mut b);
    let next_c = ticks
        .merge(&mut b, from_inner)
        .snapshot(c, |d, n| n + d)
        .hold(&mut b, 0);
    let c = c_loop.close(&mut b, next_c);
    let inner = b.constant(c.steps());
    let _inners = inners_loop.close(&mut b, inner);
}
