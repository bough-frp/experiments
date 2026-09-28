//! A split fed by its own children, built inside a `construct` body that
//! runs at a child instant: `split-fed-by-children` one level down. The
//! inner split's first events come at the children of the child instant
//! its seeds fire in. Legal. (F89: a split built at a child instant gives
//! nothing from before it existed; nothing here fires before it.)
//@ legal yes
//@ designs baseline marker marker-close
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let jobs = b.input::<Vec<u32>>();
    let _workers = jobs.split(&mut b).construct(&mut b, |b, id: u32| {
        let seeds = b.input::<Vec<u32>>();
        let (halves, halves_loop) = b.stream_loop::<Vec<u32>>();
        let items = seeds.merge(b, halves).split(b);
        let next = items.filter(|n| *n > 1).map(|n| vec![n / 2, n - n / 2]);
        halves_loop.close(b, next);
        id
    });
}
