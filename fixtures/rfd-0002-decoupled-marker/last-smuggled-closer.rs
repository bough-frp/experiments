//! Illegal: a closer smuggled into a `construct` closure through an
//! `Option` and closed at the first event, RFD 2's own example. RFD 2
//! panics at the end of the declaring scope, with the loop still open.
//@ legal no
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let spawns = b.input::<()>();
    let (mut b, _c, c_loop) = b.cell_loop::<u32>();
    let mut closer = Some(c_loop);
    let deferred = spawns.defer(&mut b);
    let _spawned = deferred.construct(&mut b, move |mut b, _| {
        let b = match closer.take() {
            Some(c_loop) => {
                let zero = b.constant(0);
                c_loop.close(b, zero).0
            }
            None => b,
        };
        (b, ())
    });
}
