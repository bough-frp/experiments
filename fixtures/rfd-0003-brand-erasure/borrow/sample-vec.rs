//! Must build and run: a sampled `Vec` of tokens, and a `Vec` of derived
//! structs. A `Vec`'s view is a `ListRef`, a borrowed slice of stored
//! elements whose `get` and `iter` brand each on the way out: a token as the
//! token, a struct as its derived `FooRef`.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Row<'g> {
    label: String,
    value: Cell<'g, u32>,
}

impl Trace for Row<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.value.trace(tracer);
    }
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, tokens, rows) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let cells: Vec<Cell<u32>> = (1..=4).map(|k| n.map(move |v| v * k).hold(b, 0)).collect();
        let rows: Vec<Row> = cells
            .iter()
            .enumerate()
            .map(|(i, &value)| Row {
                label: format!("x{}", i + 1),
                value,
            })
            .collect();
        let (never, _) = b.input::<Vec<Cell<u32>>>();
        let tokens = never.hold(b, cells);
        let (never, _) = b.input::<Vec<Row>>();
        let rows = never.hold(b, rows);
        (b.anchor(n_in), b.anchor(tokens), b.anchor(rows))
    });
    rt.send(&n_in, 5);
    rt.mutate(|b| {
        let tokens = b.open(&tokens);
        let rows = b.open(&rows);

        let list: ListRef<'_, Cell<'_, u32>> = b.sample_ref(tokens).expect("live");
        println!(
            "tokens:  len {} values {:?} third {:?}",
            list.len(),
            list.iter().map(|c| b.sample(c)).collect::<Vec<_>>(),
            list.get(2).map(|c| b.sample(c)),
        );

        let rows = b.sample_ref(rows).expect("live");
        let shown: Vec<String> = rows
            .iter()
            .map(|r| format!("{}={:?}", r.label, b.sample(r.value)))
            .collect();
        println!("rows:    {}", shown.join(" "));
        // An element's view outlives the `ListRef` it came from: both borrow
        // the stored copy for the same `'a`.
        let last = { rows }.get(3).expect("four");
        println!("last:    {} = {:?}", last.label, b.sample(last.value));
    });
}
