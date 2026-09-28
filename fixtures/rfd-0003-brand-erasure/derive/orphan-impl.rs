//! Must fail: the orphan rules. `Rebrand` is Bough's and `Celsius` is a
//! third crate's, so the user crate between them can't implement one for
//! the other, by hand or by derive.
#![forbid(unsafe_code)]
use bough::*;
use foreign::Celsius;

impl Rebrand for Celsius {
    type Of<'x> = Celsius;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Celsius {
        self.clone()
    }
    fn restore<'x>(from: &Celsius, _: Witness<'x>) -> Celsius {
        from.clone()
    }
}

fn main() {}
