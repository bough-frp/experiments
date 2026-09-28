//! Must build and run: a struct of tokens, derived. `derive-shapes.rs`
//! with the hand-written impl replaced by `#[derive(Rebrand)]`, and a field
//! in each container the api implements `Rebrand` for: `Vec`, `Option`,
//! `Box`, `HashMap`. The run prints the slots before `anchor` and after
//! `open`, and the values the opened tokens and a held copy sample.
#![forbid(unsafe_code)]
use bough::*;
use std::collections::HashMap;

#[derive(Clone, Rebrand)]
struct Panels<'g> {
    left: Cell<'g, u32>,
    history: Vec<Cell<'g, u32>>,
    pinned: Option<Cell<'g, u32>>,
    boxed: Box<Cell<'g, u32>>,
    named: HashMap<String, Cell<'g, u32>>,
}

/// `Trace` is still by hand: this probe derives `Rebrand` only.
impl Trace for Panels<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
        self.history.trace(tracer);
        self.pinned.trace(tracer);
        self.boxed.trace(tracer);
        self.named.trace(tracer);
    }
}

fn slots(p: &Panels<'_>) -> String {
    let history: Vec<u32> = p.history.iter().map(|c| c.index()).collect();
    let mut named: Vec<_> = p.named.iter().map(|(k, c)| (k.clone(), c.index())).collect();
    named.sort();
    format!(
        "left {} history {history:?} pinned {:?} boxed {} named {named:?}",
        p.left.index(),
        p.pinned.map(|c| c.index()),
        p.boxed.index(),
    )
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, panels, held) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let left = n.hold(b, 0);
        let double = n.map(|v| v * 2).hold(b, 0);
        let plus = n.map(|v| v + 100).hold(b, 100);
        let panels = Panels {
            left,
            history: vec![left, double],
            pinned: Some(plus),
            boxed: Box::new(double),
            named: HashMap::from([("plus".to_string(), plus)]),
        };
        // A cell whose value is itself a `Panels`: stored at 'static,
        // restored on every read.
        let held = n.map_to(panels.clone()).hold(b, panels.clone());
        println!("built:   {}", slots(&panels));
        (b.anchor(n_in), b.anchor(panels), b.anchor(held))
    });
    rt.send(&n_in, 3);
    rt.mutate(|b| {
        let panels = b.open(&panels);
        let held = b.sample(b.open(&held)).expect("live");
        println!("opened:  {}", slots(&panels));
        println!("held:    {}", slots(&held));
        let named: Vec<_> = panels.named.values().map(|c| b.sample(*c)).collect();
        println!(
            "values:  left {:?} history {:?} pinned {:?} boxed {:?} named {named:?}",
            b.sample(panels.left),
            panels.history.iter().map(|c| b.sample(*c)).collect::<Vec<_>>(),
            panels.pinned.map(|c| b.sample(c)),
            b.sample(*panels.boxed),
        );
    });
}
