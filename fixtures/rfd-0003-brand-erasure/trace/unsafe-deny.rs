//! Must build: `seal-derive.rs` under the `unsafe` seal, in a crate that
//! denies `unsafe_code` instead of forbidding it. The derive's `allow` lifts
//! a `deny` (a `forbid` rejects it, E0453), so the derive works, and so
//! would a hand-written `unsafe impl` under an `allow`.
#![deny(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Pair<'g> {
    left: Cell<'g, u32>,
    label: String,
}

fn main() {}
