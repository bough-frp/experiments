//! Must fail: F62. The navigation construct captures `clicks` and nothing
//! declares it; once a quiet page is selected, nothing reaches `clicks`,
//! and the construct's next run is a stale token.
#[path = "api.rs"]
mod bough;
use bough::*;

type Shown = (u32, u32);

fn screen(b: &mut Build, clicks: Shared<u32>, n: u32) -> Stream<Shown> {
    let count = clicks.accumulate(b, 0u32, |_, c| c + 1);
    clicks
        .snapshot(count, move |click, k| (n * 100 + click, *k))
        .node(b)
}

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (clicks, clicks_in) = b.input::<u32>();
        let clicks = clicks.share(b);
        let (navigate, navigate_loop) = b.stream_loop::<u32>();
        let first = screen(b, clicks, 0);
        let screens = navigate.construct(b, move |b, n| screen(b, clicks, n));
        let events = screens.hold(b, first).switch_stream(b).share(b);
        navigate_loop.close(
            b,
            events.filter_map(|(n, click)| (click == 0).then_some(n + 1)),
        );
        (clicks_in, events)
    });
}
