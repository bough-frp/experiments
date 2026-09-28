//! Must build: the ordinary switch idiom, a closure choosing among tokens
//! (F85), the candidates passed through `with`.
#![feature(auto_traits, negative_impls)]
#[path = "api.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (n, n_in) = b.input::<u32>();
        let (pick, pick_in) = b.input::<u32>();
        let latest = n.hold(b, 0);
        let doubled = latest.map_cell(b, |v| v * 2);
        let chosen = pick
            .with((doubled, latest))
            .map(|((doubled, latest), p)| if p == 1 { doubled } else { latest })
            .hold(b, latest);
        (n_in, pick_in, chosen.switch_cell(b))
    });
}
