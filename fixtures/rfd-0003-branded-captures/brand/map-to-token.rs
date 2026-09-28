//! F62's second case: `map_to(token)` with no declaration. Must fail as F62
//! states it; `map_to` traces its value (spike F94), so a build is also safe.
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
        let chosen = pick.map_to(doubled).hold(b, latest);
        let shown = chosen.switch_cell(b);
        b.anchor((n_in, pick_in, shown))
    });
}
