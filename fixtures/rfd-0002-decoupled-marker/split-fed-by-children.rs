//! A split fed by its own children: each job splits into its items, and
//! each item above one comes back as a job of two halves. The split's
//! output at a child instant `t ++ [n]` depends on nothing at `t ++ [n]`:
//! the halves it feeds back come out at that child's children. Legal, and
//! it ends because the halves shrink.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let jobs = b.input::<Vec<u32>>();
    let (halves, halves_loop) = b.stream_loop::<Vec<u32>>();
    let items = jobs.merge(&mut b, halves).split(&mut b);
    let next = items.filter(|n| *n > 1).map(|n| vec![n / 2, n - n / 2]);
    halves_loop.close(&mut b, next);
}
