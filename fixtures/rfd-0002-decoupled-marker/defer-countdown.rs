//! The countdown from Bough's `defer` documentation: each count above one
//! comes back one less in the next child instant, until the filter stops
//! it.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let starts = b.input::<u32>();
    let (counts, counts_loop) = b.stream_loop::<u32>();
    let again = counts.filter(|n| *n > 1).map(|n| n - 1).defer(&mut b);
    let counts = starts.merge(&mut b, again);
    counts_loop.close(&mut b, counts);
}
