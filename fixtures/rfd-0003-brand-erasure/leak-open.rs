//! Wanted to fail; builds and runs in the default design: question 3. A
//! `construct` closure captures an `Anchored` and reopens it. The anchor is
//! a guard hidden in graph state (RFD 3's exception): it roots a cell that
//! reaches the node holding the closure, through the loop, so after every
//! guard the program holds is dropped, a collection frees nothing. The same
//! graph without the capture is collected. The run prints both.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

fn build(rt: &mut Runtime, capture: bool) -> Anchored<Input<'static, u32>> {
    rt.mutate(move |b| {
        let (navigate, navigate_loop) = b.stream_loop::<u32>();
        let shown = navigate.hold(b, 0);
        let shown = b.anchor(shown);
        let (opens, opens_in) = b.input::<u32>();
        let made = if capture {
            opens.construct(b, move |b, n| {
                let shown = b.open(&shown);
                shown.map_cell(b, move |v| v + n)
            })
        } else {
            drop(shown);
            opens.construct(b, |b, n| {
                let (s, _) = b.input::<u32>();
                s.hold(b, n)
            })
        };
        navigate_loop.close(b, made.map(|_| 1u32));
        b.anchor(opens_in)
    })
}

fn main() {
    for capture in [true, false] {
        let mut rt = Runtime::new();
        let opens_in = build(&mut rt, capture);
        let built = rt.live_nodes();
        drop(opens_in);
        let freed = rt.collect();
        println!(
            "{}: {built} nodes built, {freed} freed once every held guard is dropped, {} live",
            if capture {
                "captured anchor"
            } else {
                "no capture     "
            },
            rt.live_nodes()
        );
    }
}
