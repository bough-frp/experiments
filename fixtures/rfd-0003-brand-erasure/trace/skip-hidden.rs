//! Builds, and shouldn't: a `'static` token inside a type that implements
//! neither `Rebrand` nor `Trace`, skipped. The check asks the type's impls,
//! and this type has none; the token is as hidden as in `skip-static.rs`.
//! The value would come from the same stash, so this is only type-checked.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone)]
struct Hidden {
    count: Cell<'static, u32>,
}

#[derive(Clone, Rebrand, Trace)]
struct Pinned {
    #[rebrand(skip)]
    hidden: Hidden,
}

fn main() {}
