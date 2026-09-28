//! Must build: RFD 4's screens example and F62's navigation loop, in
//! outline. Each screen is built with its own input, which goes out to I/O
//! code anchored; the navigation construct captures `clicks`, imports it into
//! its era, and declares it.
#[path = "api.rs"]
mod bough;
use bough::*;

type Shown = (u32, u32);

fn screen<'e>(b: &mut Build<'e>, clicks: Shared<'e, u32>, n: u32) -> Stream<'e, Shown> {
    let count = clicks.accumulate(b, 0u32, |_, c| c + 1);
    clicks
        .snapshot(count, move |click, k| (n * 100 + click, *k))
        .node(b)
}

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        // RFD 4: a screen and its own input, split with unzip.
        let (opens, opens_in) = b.input::<u32>();
        let made = opens.construct::<(Stream<'static, u32>, Anchored<Input<'static, u32>>), _>(
            b,
            |b, id| {
                let (keys, keys_in) = b.input::<u32>();
                let screen = keys.map(move |key| id * 100 + key).node(b);
                (screen, b.anchor(keys_in))
            },
        );
        let (screens, inputs) = made.unzip(b);
        let idle = b.input::<u32>().0;
        let shown = screens.hold(b, idle).switch_stream(b).share(b);

        // F62: the navigation loop, whose construct captures `clicks`.
        let (clicks, clicks_in) = b.input::<u32>();
        let clicks = clicks.share(b);
        let (navigate, navigate_loop) = b.stream_loop::<u32>();
        let first = screen(b, clicks, 0);
        let screens = navigate.construct::<Stream<'static, Shown>, _>(b, move |b, n| {
            let clicks = b.import(clicks);
            screen(b, clicks, n)
        });
        b.depends(&screens, &[&clicks]);
        let events = screens.hold(b, first).switch_stream(b).share(b);
        navigate_loop.close(
            b,
            events.filter_map(|(n, click)| (click == 0).then_some(n + 1)),
        );
        ((opens_in, clicks_in), (shown, events), inputs.share(b))
    });
}
