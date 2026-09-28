//! Builds, and shouldn't: the api's `Leaf`, the skip as a wrapper, around a
//! `'static` token. `Leaf<T>` is `Rebrand` for any `T: Clone + 'static` and
//! traces nothing, and its impl is generic, so the derive's check (which
//! needs a concrete type) can't be put there. Only type-checked, as
//! `skip-hidden.rs`.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone, Rebrand, Trace)]
struct Pinned {
    count: Leaf<Cell<'static, u32>>,
}

fn main() {}
