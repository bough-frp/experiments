//! Must fail: inside a construct, a switch closure captures tokens the
//! construct itself minted, and nothing declares them.
#[path = "api.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (opens, opens_in) = b.input::<u32>();
        let made = opens.construct(b, |b, id| {
            let (n, n_in) = b.input::<u32>();
            let latest = n.hold(b, id);
            let doubled = latest.map_cell(b, |v| v * 2);
            let chosen = latest.map_cell(b, move |v| if v % 2 == 0 { doubled } else { latest });
            (chosen.switch_cell(b), b.anchor(n_in))
        });
        let made = made.share(b);
        (opens_in, made)
    });
}
