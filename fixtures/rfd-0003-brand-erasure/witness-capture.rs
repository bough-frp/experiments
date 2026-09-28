//! Must fail: a type whose `Rebrand` keeps the witness it is given, so that
//! `open` hands user code a `Witness<'g>`, which a graph closure captures to
//! rebrand an anchored token at run time without `open`.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

#[derive(Clone, Copy)]
struct Grab<'g> {
    witness: Option<Witness<'g>>,
}

impl Trace for Grab<'_> {
    fn trace(&self, _: &mut Tracer) {}
}

impl<'g> Rebrand for Grab<'g> {
    type Of<'x> = Grab<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Grab<'x> {
        Grab { witness: Some(to) }
    }
    fn restore<'x>(_: &Grab<'x>, _: Witness<'x>) -> Self {
        Grab { witness: None }
    }
}

fn main() {
    let mut rt = Runtime::new();
    let grab = rt.mutate(|b| b.anchor(Grab { witness: None }));
    let _ = rt.mutate(move |b| {
        let witness = b.open(&grab).witness.expect("kept by rebrand");
        let (opens, opens_in) = b.input::<u32>();
        let made = opens.construct(b, move |b, n| {
            let (s, _) = b.input::<u32>();
            let _w = witness;
            s.hold(b, n)
        });
        b.anchor((opens_in, made.map(|_| 0u32).hold(b, 0)))
    });
}
