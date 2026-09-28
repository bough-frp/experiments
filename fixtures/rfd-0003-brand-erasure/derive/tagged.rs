//! Must build and run: generic types, derived. `Tagged<T>` has a type
//! parameter and no brand, so the derive adds `T: Rebrand` and writes
//! `Of<'x> = Tagged<T::Of<'x>>`; `Keyed<'g, T>` has both, and a generic
//! field in each container, so its `Of` renames the brand too. Both carry
//! tokens through `anchor` and `open`, and brand-free values through a cell.
#![forbid(unsafe_code)]
use bough::*;
use std::collections::HashMap;

#[derive(Clone, Rebrand)]
struct Tagged<T> {
    tag: String,
    value: T,
}

#[derive(Clone, Rebrand)]
struct Keyed<'g, T> {
    source: Cell<'g, u32>,
    all: Vec<T>,
    first: Option<T>,
    boxed: Box<T>,
    by_name: HashMap<String, T>,
}

impl<T: Trace> Trace for Tagged<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.value.trace(tracer);
    }
}

impl<T: Trace> Trace for Keyed<'_, T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.source.trace(tracer);
        self.all.trace(tracer);
        self.first.trace(tracer);
        self.boxed.trace(tracer);
        self.by_name.trace(tracer);
    }
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, tagged, keyed, labels) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let total = n.accumulate(b, 0u32, |v, sum| sum + v);
        let tagged = Tagged {
            tag: "total".to_string(),
            value: total,
        };
        let keyed = Keyed {
            source: total,
            all: vec![tagged.clone()],
            first: Some(tagged.clone()),
            boxed: Box::new(tagged.clone()),
            by_name: HashMap::from([("total".to_string(), tagged.clone())]),
        };
        // `Tagged<u32>` has no token, but it is generic, so it has no view:
        // every read of this cell restores a copy.
        let labels = n
            .map(|v| Tagged {
                tag: "last".to_string(),
                value: v,
            })
            .hold(b, Tagged {
                tag: "none".to_string(),
                value: 0,
            });
        (b.anchor(n_in), b.anchor(tagged), b.anchor(keyed), b.anchor(labels))
    });
    rt.send(&n_in, 3);
    rt.send(&n_in, 4);
    rt.mutate(|b| {
        let tagged = b.open(&tagged);
        let keyed = b.open(&keyed);
        println!("tagged:  {} = {:?}", tagged.tag, b.sample(tagged.value));
        println!(
            "keyed:   source {:?} all {:?} first {:?} boxed {:?} by_name {:?}",
            b.sample(keyed.source),
            keyed.all.iter().map(|t| b.sample(t.value)).collect::<Vec<_>>(),
            keyed.first.as_ref().map(|t| b.sample(t.value)),
            b.sample(keyed.boxed.value),
            keyed.by_name.get("total").map(|t| b.sample(t.value)),
        );
        let label = b.sample(b.open(&labels)).expect("live");
        println!("labels:  {} = {}", label.tag, label.value);
    });
}
