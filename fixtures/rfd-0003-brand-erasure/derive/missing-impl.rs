//! Must fail: a derived struct with a field whose type doesn't implement
//! `Rebrand`, a local type its author forgot to derive it for. The error
//! is the one a user sees, at the field.
#![forbid(unsafe_code)]
use bough::*;

struct Label(String);

#[derive(Rebrand)]
struct Row<'g> {
    count: Cell<'g, u32>,
    label: Label,
}

fn main() {}
