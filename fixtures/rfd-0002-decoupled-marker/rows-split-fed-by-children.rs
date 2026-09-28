//! `split-fed-by-children` under rows.
//@ legal yes
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let jobs = b.input::<Vec<u32>>();
    let (halves, halves_loop) = b.stream_loop::<Vec<u32>, L0, Empty>();
    let items = jobs.merge(&mut b, halves).split(&mut b);
    let next = items.filter(|n| *n > 1).map(|n| vec![n / 2, n - n / 2]);
    halves_loop.close(&mut b, next);
}
