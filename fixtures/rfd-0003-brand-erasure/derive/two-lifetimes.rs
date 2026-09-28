//! Must fail: a type with a lifetime besides its brand. The derive refuses
//! it with a message of its own: a value is stored at brand `'static`, so it
//! can't hold a borrow.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Borrowing<'g, 'a> {
    count: Cell<'g, u32>,
    name: &'a str,
}

fn main() {}
