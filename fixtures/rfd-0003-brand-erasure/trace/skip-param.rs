//! Must fail: a skipped field of a type parameter, which may be a token.
//! The committed derive refused it as `T: Clone is not satisfied`
//! (`derive/skip-param.rs`); the checked one says why it matters.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone, Rebrand, Trace)]
struct Wrapped<T> {
    #[rebrand(skip)]
    value: T,
}

fn main() {}
