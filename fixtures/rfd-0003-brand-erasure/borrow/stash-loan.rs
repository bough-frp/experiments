//! Must fail: a hand-written `Borrow` impl keeps the `Loan` it is handed, to
//! call `borrow` later on a stored token and get it at any brand. The loan
//! is scoped to the call's `'a`, which an impl can't name as `'static`.
#![forbid(unsafe_code)]
use bough::*;

thread_local! {
    static STASH: std::cell::Cell<Option<Loan<'static>>> = const { std::cell::Cell::new(None) };
}

struct Pair<'g> {
    left: Cell<'g, u32>,
    label: String,
}

impl Trace for Pair<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
    }
}

impl<'g> Rebrand for Pair<'g> {
    type Of<'x> = Pair<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Pair<'x> {
        Pair {
            left: self.left.rebrand(to),
            label: self.label.clone(),
        }
    }
    fn restore<'x>(from: &Pair<'x>, at: Witness<'x>) -> Self {
        Pair {
            left: Rebrand::restore(&from.left, at),
            label: from.label.clone(),
        }
    }
}

struct PairRef<'a, 'g> {
    left: Cell<'g, u32>,
    label: &'a String,
}

impl<'g> Borrow for Pair<'g> {
    type Ref<'a> = PairRef<'a, 'g>;
    fn borrow<'a>(from: &'a Pair<'static>, at: Loan<'a>) -> PairRef<'a, 'g> {
        STASH.set(Some(at));
        PairRef {
            left: <Cell<'g, u32> as Borrow>::borrow(&from.left, at),
            label: &from.label,
        }
    }
}

fn main() {}
