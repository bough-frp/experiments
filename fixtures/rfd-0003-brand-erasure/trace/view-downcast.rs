//! Must fail: `view` for a generic type by a `'static` downcast, the
//! stored `Tagged<T::Of<'static>>` against `Self`. `Any` needs `Self:
//! 'static`, so `T: 'static`, which the impl can't require: `Tagged<T>` must
//! be `Rebrand` for `T = Cell<'g, u32>` too, and a stricter bound on the
//! method than the trait's is refused (`view-marker.rs`).
#![forbid(unsafe_code)]
use bough::*;
use std::any::Any;

#[derive(Clone)]
struct Tagged<T> {
    tag: String,
    value: T,
}

impl<T: Rebrand> Rebrand for Tagged<T> {
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
        (from as &dyn Any).downcast_ref::<Self>()
    }
}

fn main() {}
