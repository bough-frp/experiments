//! Must fail: F85. A switch closure captures the tokens it chooses among.
#[path = "api.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let (pick, pick_in) = b.input::<u32>();
        let latest = n.hold(b, 0);
        let doubled = latest.map_cell(b, |v| v * 2);
        let chosen = pick
            .map(move |p| if p == 1 { doubled } else { latest })
            .hold(b, latest);
        let shown = chosen.switch_cell(b);
        b.anchor((n_in, pick_in, shown))
    });
}
