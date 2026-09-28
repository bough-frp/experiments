//! Must fail: `#[rebrand(skip)]` on a field of a type parameter. `T` would
//! have to be `T::Of<'x>`, which the impl can't prove for every `T`.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Wrapped<T> {
    #[rebrand(skip)]
    value: T,
}

fn main() {}
