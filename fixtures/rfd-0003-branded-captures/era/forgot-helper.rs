//! Must fail: F62 as most code would meet it, the navigation loop built in
//! a helper function, whose construct captures `clicks` undeclared.
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

fn navigation(b: &mut Build<'static>, clicks: Shared<'static, u32>) -> Shared<'static, Shown> {
    let (navigate, navigate_loop) = b.stream_loop::<u32>();
    let first = screen(b, clicks, 0);
    let screens = navigate.construct::<Stream<'static, Shown>, _>(b, move |b, n| {
        let clicks = b.import(clicks);
        screen(b, clicks, n)
    });
    let events = screens.hold(b, first).switch_stream(b).share(b);
    navigate_loop.close(
        b,
        events.filter_map(|(n, click)| (click == 0).then_some(n + 1)),
    );
    events
}

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (clicks, clicks_in) = b.input::<u32>();
        let clicks = clicks.share(b);
        let events = navigation(b, clicks);
        (clicks_in, events)
    });
}
