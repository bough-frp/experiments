//! Must build: the derive under a seal. `#[derive(Rebrand)]` writes the
//! seal's impl beside `Rebrand`'s, in this crate, which forbids `unsafe`.
//! Built against the safe seal, and the `unsafe` one with and without the
//! derive's `#[allow(unsafe_code)]`.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Pair<'g> {
    left: Cell<'g, u32>,
    label: String,
}

fn main() {
    mutate(|s| {
        let pair = Pair {
            left: s.cell(),
            label: "pair".to_string(),
        };
        let stored = s.store(&pair);
        s.read(&stored, |p: &Pair| {
            println!("{} {}", p.label, p.left.index())
        });
    });
}
