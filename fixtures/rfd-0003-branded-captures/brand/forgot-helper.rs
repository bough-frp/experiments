//! Must fail: F62 as most code would meet it, the navigation loop built in
//! a helper function, whose construct captures `clicks` undeclared.
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

fn navigation<'g>(b: &mut Build<'g>, clicks: Shared<'g, u32>) -> Shared<'g, Shown> {
    let (navigate, navigate_loop) = b.stream_loop::<u32>();
    let first = screen(b, clicks, 0);
    let screens = navigate.construct(b, move |b, n| screen(b, clicks, n));
    let events = screens.hold(b, first).switch_stream(b).share(b);
    navigate_loop.close(
        b,
        events.filter_map(|(n, click)| (click == 0).then_some(n + 1)),
    );
    events
}

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (clicks, clicks_in) = b.input::<u32>();
        let clicks = clicks.share(b);
        let events = navigation(b, clicks);
        b.anchor((clicks_in, events))
    });
}
