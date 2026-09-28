//! F22: the countdown with no filter. Every event comes back in the next
//! child instant, so the first send never returns. RFD 5 says it is legal:
//! the loop rule is about same-instant cycles and doesn't bound a
//! transaction. It must build.
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
    let again = counts.map(|n| n + 1).defer(&mut b);
    let counts = starts.merge(&mut b, again);
    counts_loop.close(&mut b, counts);
}
