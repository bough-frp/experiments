//! Must build and run: the write view of a struct of tokens, derived.
//! `Panels<'g>` holds a token, a `Vec` of derived rows, an `Option` of a
//! token and a `String`; `update` hands the stored copy to a closure as a
//! `Place`, whose `as_mut` is the derived `PanelsMut<'_, 'g>`. The closure
//! sets a token, edits a string in place, and pushes to, retains, sorts and
//! edits the rows through `ListMut`, whose closures see each row's view at
//! the writer's brand. Nothing hands out the stored `Vec<Row<'static>>`.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Row<'g> {
    label: String,
    value: Cell<'g, u32>,
}

#[derive(Rebrand)]
struct Panels<'g> {
    left: Cell<'g, u32>,
    rows: Vec<Row<'g>>,
    pinned: Option<Cell<'g, u32>>,
    title: String,
}

impl Trace for Row<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.value.trace(tracer);
    }
}

impl Trace for Panels<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.left.trace(tracer);
        self.rows.trace(tracer);
        self.pinned.trace(tracer);
    }
}

fn show<'g>(b: &Build<'g>, p: PanelsRef<'_, 'g>) -> String {
    let rows: Vec<String> = p
        .rows
        .iter()
        .map(|r| format!("{}={:?}", r.label, b.sample(r.value)))
        .collect();
    format!(
        "{}: left {:?} rows [{}] pinned {:?}",
        p.title,
        b.sample(p.left),
        rows.join(" "),
        p.pinned.map(|c| b.sample(c)),
    )
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, cells, panels) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let cells: Vec<Cell<u32>> = (1..=4).map(|k| n.map(move |v| v * k).hold(b, 0)).collect();
        let row = |label: &str, value| Row {
            label: label.to_string(),
            value,
        };
        let init = Panels {
            left: cells[0],
            rows: vec![row("c", cells[0]), row("a", cells[1]), row("d", cells[2])],
            pinned: Some(cells[3]),
            title: "before".to_string(),
        };
        let (never, _) = b.input::<Panels>();
        let panels = never.hold(b, init);
        (b.anchor(n_in), b.anchor(cells), b.anchor(panels))
    });
    rt.send(&n_in, 10);
    rt.mutate(|b| {
        let cells = b.open(&cells);
        let panels = b.open(&panels);
        println!("before:  {}", show(b, b.sample_ref(panels).expect("live")));
        b.update(panels, |mut place| {
            let mut p = place.as_mut();
            p.left.set(cells[3]);
            p.title.clear();
            p.title.push_str("after");
            p.pinned.set(None);
            p.rows.push(Row {
                label: "b".to_string(),
                value: cells[3],
            });
            // Closures over views at the writer's brand.
            p.rows.retain(|r| r.label != "d");
            p.rows.sort_by(|x, y| x.label.cmp(y.label));
            for mut r in p.rows.iter_mut() {
                r.label.make_ascii_uppercase();
            }
            if let Some(mut first) = p.rows.get_mut(0) {
                first.value.set(cells[2]);
            }
        })
        .expect("live");
        println!("after:   {}", show(b, b.sample_ref(panels).expect("live")));
        // A whole value replaced through the root `Place`.
        b.update(panels, |mut place| {
            let old = place.get();
            place.set(Panels {
                left: old.left,
                rows: Vec::new(),
                pinned: Some(cells[0]),
                title: "reset".to_string(),
            });
        })
        .expect("live");
        println!("reset:   {}", show(b, b.sample_ref(panels).expect("live")));
    });
}
