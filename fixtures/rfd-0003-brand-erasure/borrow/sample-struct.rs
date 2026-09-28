//! Must build and run: RFD 4's `sample` as a view. `Panels<'g>` is a struct
//! of tokens in each container the api views (`Vec`, `Option`), a `String`
//! and a leaf that counts its restores, held in a cell. The derive writes
//! `PanelsRef<'a, 'g>`; `sample_ref` returns one over the stored
//! `Panels<'static>`, its tokens at the reader's brand, and the run counts
//! the restores a `sample` and a `sample_ref` each cost.
#![forbid(unsafe_code)]
use bough::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static RESTORES: AtomicUsize = AtomicUsize::new(0);

/// A brand-free leaf, hand-written, that counts its restores.
struct Counted(u32);

impl Rebrand for Counted {
    type Of<'x> = Counted;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Counted {
        Counted(self.0)
    }
    fn restore<'x>(from: &Counted, _: Witness<'x>) -> Counted {
        RESTORES.fetch_add(1, Ordering::Relaxed);
        Counted(from.0)
    }
}

impl Borrow for Counted {
    type Ref<'a> = &'a Counted;
    fn borrow<'a>(from: &'a Counted, _: Loan<'a>) -> &'a Counted {
        from
    }
}

/// The derive writes both views, so a field needs both.
impl BorrowMut for Counted {
    type Mut<'a> = &'a mut Counted;
    fn borrow_mut<'a>(from: &'a mut Counted, _: Loan<'a>) -> &'a mut Counted {
        from
    }
}

impl Trace for Counted {
    fn trace(&self, _: &mut Tracer) {}
}

#[derive(Rebrand)]
struct Panels<'g> {
    left: Cell<'g, u32>,
    history: Vec<Cell<'g, u32>>,
    pinned: Option<Cell<'g, u32>>,
    title: String,
    reads: Counted,
}

impl Trace for Panels<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
        self.history.trace(tracer);
        self.pinned.trace(tracer);
    }
}

/// The generated type, named in a signature: tokens at `'g`, which the
/// caller's `Build<'g>` samples.
fn show<'g>(b: &Build<'g>, p: PanelsRef<'_, 'g>) -> String {
    format!(
        "{} left {:?} history {:?} pinned {:?} leaf {}",
        p.title,
        b.sample(p.left),
        p.history.iter().map(|c| b.sample(c)).collect::<Vec<_>>(),
        p.pinned.map(|c| b.sample(c)),
        p.reads.0,
    )
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, panels) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let left = n.hold(b, 0);
        let double = n.map(|v| v * 2).hold(b, 0);
        let plus = n.map(|v| v + 100).hold(b, 100);
        // A cell holding the struct: an input never sent, held.
        let (never, _) = b.input::<Panels>();
        let panels = never.hold(
            b,
            Panels {
                left,
                history: vec![left, double],
                pinned: Some(plus),
                title: "panels".to_string(),
                reads: Counted(7),
            },
        );
        (b.anchor(n_in), b.anchor(panels))
    });
    rt.send(&n_in, 3);
    rt.mutate(|b| {
        let panels = b.open(&panels);

        RESTORES.store(0, Ordering::Relaxed);
        let copy = b.sample(panels).expect("live");
        let by_sample = RESTORES.swap(0, Ordering::Relaxed);
        let view = b.sample_ref(panels).expect("live");
        let by_view = RESTORES.swap(0, Ordering::Relaxed);

        // `Copy`, as `&A` is: passed on and read again.
        let again = view;
        println!("view:    {}", show(b, view));
        println!("again:   {}", show(b, again));
        println!(
            "copy:    left {:?} history {:?}",
            b.sample(copy.left),
            copy.history
                .iter()
                .map(|c| b.sample(*c))
                .collect::<Vec<_>>(),
        );
        // Two samples in one expression, as RFD 4's `&A` allows: both
        // borrow `b` shared.
        let sum = b.sample_ref(view.left).expect("live")
            + b.sample_ref(view.history.get(1).expect("two"))
                .expect("live");
        println!("sum:     left + double = {sum}");
        println!("restores: sample {by_sample}, sample_ref {by_view}");
    });
}
