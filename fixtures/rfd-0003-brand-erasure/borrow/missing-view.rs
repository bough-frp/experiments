//! Must fail: a derived struct whose field has `Rebrand` but no views, as a
//! hand-written leaf written before the views existed would. The derive
//! writes `Borrow` and `BorrowMut` for every `Rebrand`, so each field needs
//! both; this probe records where the error lands.
#![forbid(unsafe_code)]
use bough::*;

/// A brand-free leaf with `Rebrand` alone.
struct Celsius(f64);

impl Rebrand for Celsius {
    type Of<'x> = Celsius;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Celsius {
        Celsius(self.0)
    }
    fn restore<'x>(from: &Celsius, _: Witness<'x>) -> Celsius {
        Celsius(from.0)
    }
}

#[derive(Rebrand)]
struct Reading<'g> {
    sensor: Cell<'g, u32>,
    temperature: Celsius,
}

fn main() {}
