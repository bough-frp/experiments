//! Must fail: F85. A switch closure chooses among tokens it captured, and
//! nothing declares them; the first move to a candidate nothing else
//! reaches is a stale token.
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
            .map(move |p| if p == 1 { doubled } else { latest })
            .hold(b, latest);
        (n_in, pick_in, chosen.switch_cell(b))
    });
}
