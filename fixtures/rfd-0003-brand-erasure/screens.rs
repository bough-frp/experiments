//! Must build: RFD 4's screens example and F62's navigation loop, in
//! outline, as in the earlier probe, against the safe runtime. The screen's
//! input is anchored inside the `construct`, as RFD 4 writes it.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

type Shown = (u32, u32);

fn screen<'g>(b: &mut Build<'g>, clicks: Shared<'g, u32>, n: u32) -> Stream<'g, Shown> {
    let count = clicks.accumulate(b, 0u32, |_, c| c + 1);
    clicks
        .snapshot(count, move |click, k| (n * 100 + click, *k))
        .node(b)
}

fn main() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        // RFD 4: a screen and its own input, split with unzip.
        let (opens, opens_in) = b.input::<u32>();
        let made = opens.construct(b, |b, id| {
            let (keys, keys_in) = b.input::<u32>();
            let screen = keys.map(move |key| id * 100 + key).node(b);
            (screen, b.anchor(keys_in))
        });
        let (screens, inputs) = made.unzip(b);
        let idle = b.input::<u32>().0;
        let shown = screens.hold(b, idle).switch_stream(b).share(b);
        let inputs = inputs.share(b);
        let _inputs_to_io = b.listen(inputs, |keys_in| drop(keys_in));

        // F62: the navigation loop.
        let (clicks, clicks_in) = b.input::<u32>();
        let clicks = clicks.share(b);
        let (navigate, navigate_loop) = b.stream_loop::<u32>();
        let first = screen(b, clicks, 0);
        let screens = navigate
            .with(clicks)
            .construct(b, |b, (clicks, n)| screen(b, clicks, n));
        let events = screens.hold(b, first).switch_stream(b).share(b);
        navigate_loop.close(
            b,
            events.filter_map(|(n, click)| (click == 0).then_some(n + 1)),
        );
        b.anchor(((opens_in, clicks_in), shown, events))
    });
}
