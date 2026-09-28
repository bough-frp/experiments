//! Must build: the switch idiom, a closure choosing among tokens (F85), the
//! candidates passed through `with`. The closure returns a token, which the
//! engine stores at `'static` and restores at the closure's brand.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

fn main() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let (pick, pick_in) = b.input::<u32>();
        let latest = n.hold(b, 0);
        let doubled = latest.map_cell(b, |v| v * 2);
        let chosen = pick
            .with((doubled, latest))
            .map(|((doubled, latest), p)| if p == 1 { doubled } else { latest })
            .hold(b, latest);
        let shown = chosen.switch_cell(b);
        b.anchor((n_in, pick_in, shown))
    });
}
