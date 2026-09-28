//! Illegal: `switch-smuggle` at a child instant, through `construct`. A
//! stream loop carries cell tokens as data; each one, deferred to
//! `t ++ [0]`, is handed to a `construct` that returns its steps as the
//! switch's next inner. The loop closes with a stream of `total`'s own
//! token, so the switch moves to `total`'s steps, and `total` depends on
//! itself in the same instant. The engine refuses it at that move, at run
//! time. The defer only puts the closure at a child instant: the switch
//! drops the outer's mark with or without it.
//@ legal no
//@ designs baseline marker marker-close
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let go = b.input::<()>();
    let (tokens, tokens_loop) = b.stream_loop::<Cell<u32>>();
    let first = b.constant(0);
    let latest = tokens.defer(&mut b).hold(&mut b, first);
    let screen = latest.construct(&mut b, |_, total: &Cell<u32>| total.steps());
    let from_inner = screen.switch_stream(&mut b);
    let total = ticks.merge(&mut b, from_inner).hold(&mut b, 0);
    tokens_loop.close(&mut b, go.map(move |_| total));
}
