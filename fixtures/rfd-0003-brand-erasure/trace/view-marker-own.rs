//! Must fail: `view-marker-impl.rs` with the impl a brand-free `T` allows,
//! `Of<'x> = Tagged<T>`, cloned across brands (`T: 'static` too, which
//! `Of<'x>: 'x` needs and the equality doesn't give). It builds, and `Tagged<u32>`
//! gets a copy-free `view`, but `Tagged<Cell<'g, u32>>` is no longer
//! `Rebrand` at all: the first error is where it is anchored. A generic type
//! is copy-free or holds tokens, not both.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone)]
struct Tagged<T> {
    tag: String,
    value: T,
}

impl<T: Rebrand<Of<'static> = T> + Clone + 'static> Rebrand for Tagged<T> {
    type Of<'x> = Tagged<T>;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Tagged<T> {
        self.clone()
    }
    fn restore<'x>(from: &Tagged<T>, _: Witness<'x>) -> Self {
        from.clone()
    }
    fn view<'a>(from: &'a Self::Of<'static>) -> Option<&'a Self> {
        Some(from)
    }
}

impl<T: Trace> Trace for Tagged<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.value.trace(tracer);
    }
}

fn main() {
    let mut rt = Runtime::new();
    rt.mutate(|b| {
        let (n, _) = b.input::<u32>();
        let cell = n.hold(b, 0);
        let plain = b.anchor(Tagged {
            tag: "plain".to_string(),
            value: 1u32,
        });
        let token = b.anchor(Tagged {
            tag: "token".to_string(),
            value: cell,
        });
        let _ = (plain, token);
    });
}
