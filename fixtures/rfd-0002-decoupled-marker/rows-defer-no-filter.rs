//! `defer-no-filter`, F22, under rows. Legal, and it never ends.
//@ legal yes
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let starts = b.input::<u32>();
    let (counts, counts_loop) = b.stream_loop::<u32, L0, Empty>();
    let again = counts.map(|n| n + 1).defer(&mut b);
    let counts = starts.merge(&mut b, again);
    counts_loop.close(&mut b, counts);
}
