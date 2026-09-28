//! Must build and run: another crate's type, `foreign::Celsius`, which
//! implements no Bough trait, in a stream and in a derived struct. The
//! orphan rules forbid `impl Rebrand for Celsius` here (`orphan-impl.rs`),
//! so it goes in wrapped: as `Leaf<Celsius>`, the api's wrapper, in a
//! stream or a field, or as a field marked `#[rebrand(skip)]`, the same
//! thing at the field, with no wrapper to unwrap.
#![forbid(unsafe_code)]
use bough::*;
use foreign::Celsius;

#[derive(Clone, Rebrand)]
struct Reading<'g> {
    sensor: Cell<'g, u32>,
    wrapped: Leaf<Celsius>,
    #[rebrand(skip)]
    skipped: Celsius,
}

impl Trace for Reading<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.sensor.trace(tracer);
    }
}

fn main() {
    let mut rt = Runtime::new();
    let (t_in, reading, latest) = rt.mutate(|b| {
        // An event of a foreign type: `Leaf` in the stream's type.
        let (t, t_in) = b.input::<Leaf<Celsius>>();
        let t = t.share(b);
        let sensor = t.map(|c| (c.0.0 * 10.0) as u32).hold(b, 0);
        let reading = Reading {
            sensor,
            wrapped: Leaf(Celsius(-1.0)),
            skipped: Celsius(-2.0),
        };
        // The token reaches the closure as data, through `with`.
        let latest = t
            .with(sensor)
            .map(|(sensor, c)| Reading {
                sensor,
                wrapped: c.clone(),
                skipped: c.0.clone(),
            })
            .hold(b, reading.clone());
        (b.anchor(t_in), b.anchor(reading), b.anchor(latest))
    });
    rt.send(&t_in, Leaf(Celsius(21.5)));
    rt.mutate(|b| {
        let reading = b.open(&reading);
        let latest = b.sample(b.open(&latest)).expect("live");
        println!(
            "anchored: sensor {:?} wrapped {:?} skipped {:?}",
            b.sample(reading.sensor),
            reading.wrapped.0,
            reading.skipped
        );
        println!(
            "latest:   sensor {:?} wrapped {:?} skipped {:?}",
            b.sample(latest.sensor),
            *latest.wrapped,
            latest.skipped
        );
    });
}
