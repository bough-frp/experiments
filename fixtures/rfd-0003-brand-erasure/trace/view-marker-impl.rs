//! Must fail: the marker bound on the whole impl instead of the method, with
//! the impl the derive writes. With `T: Rebrand<Of<'static> = T>` in scope,
//! rustc takes that clause for `T::Of` at every lifetime, so the derive's
//! own `Of<'x> = Tagged<T::Of<'x>>` no longer type-checks: the first error
//! is in the impl. `view-marker-own.rs` writes the impl a brand-free `T`
//! allows instead.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone)]
struct Tagged<T> {
    tag: String,
    value: T,
}

impl<T: Rebrand<Of<'static> = T>> Rebrand for Tagged<T> {
    type Of<'x> = Tagged<T::Of<'x>>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        Tagged {
            tag: self.tag.clone(),
            value: self.value.rebrand(to),
        }
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        Tagged {
            tag: from.tag.clone(),
            value: T::restore(&from.value, at),
        }
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
