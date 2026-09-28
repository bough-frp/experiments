//! Must fail: a skipped token whose event type is not `Rebrand`, so the
//! token isn't either (`Cell<'g, A>: Rebrand` needs `A: Rebrand`). It is
//! still `Trace` (a token traces its id whatever `A` is), and that is what
//! the check catches.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone)]
struct Celsius(f64);

#[derive(Clone, Rebrand, Trace)]
struct Pinned {
    #[rebrand(skip)]
    reading: Option<Cell<'static, Celsius>>,
}

fn main() {}
