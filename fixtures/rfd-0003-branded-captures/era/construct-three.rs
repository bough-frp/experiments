//! Must build: a `construct` whose closure captures three tokens, imports
//! them into its era, and declares them.
#[path = "api.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (clicks, clicks_in) = b.input::<u32>();
        let clicks = clicks.share(b);
        let scale = clicks.hold(b, 1);
        let offset = clicks.map(|c| c + 10).hold(b, 10);
        let (opens, opens_in) = b.input::<u32>();
        let made = opens.construct::<Cell<'static, u32>, _>(b, move |b, id| {
            let (clicks, scale, offset) = (b.import(clicks), b.import(scale), b.import(offset));
            clicks
                .snapshot(scale, move |c, s| id + c * s)
                .snapshot(offset, |v, o| v + o)
                .hold(b, 0)
        });
        b.depends(&made, &[&clicks, &scale, &offset]);
        let made = made.hold(b, scale);
        (clicks_in, opens_in, made)
    });
}
