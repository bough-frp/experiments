//! Must fail, and is what a seal costs: a hand-written `Rebrand` that stashes
//! nothing, for a leaf that counts its restores (`derive/view.rs`'s
//! `Counted`). Under a seal no impl is hand-written, a correct one included.
#![forbid(unsafe_code)]
use bough::*;

static RESTORES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct Counted(u32);

impl Rebrand for Counted {
    type Of<'x> = Counted;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Counted {
        Counted(self.0)
    }
    fn restore<'x>(from: &Counted, _: Witness<'x>) -> Counted {
        RESTORES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Counted(from.0)
    }
    fn view(from: &Counted) -> Option<&Counted> {
        Some(from)
    }
}

fn main() {}
