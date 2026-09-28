//! Must build and run: what a brand-free value keeps. `Stats` has no brand
//! and no type parameter, so the derive makes its views `&Stats` and
//! `&mut Stats`: `Debug`, functions taking `&Stats`, and the whole `Vec` API
//! on its field. A `Vec<u32>` held on its own is viewed through `ListRef`
//! and `ListMut`, whose `as_slice` and `as_mut_vec` hand out the stored
//! `Vec` whole, because a brand-free element is its own stored copy.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Debug, Rebrand)]
struct Stats {
    count: u32,
    seen: Vec<u32>,
}

impl Trace for Stats {
    fn trace(&self, _: &mut Tracer) {}
}

fn mean(s: &Stats) -> u32 {
    s.seen.iter().sum::<u32>() / s.seen.len().max(1) as u32
}

fn main() {
    let mut rt = Runtime::new();
    let (stats, numbers) = rt.mutate(|b| {
        let (never, _) = b.input::<Stats>();
        let stats = never.hold(
            b,
            Stats {
                count: 4,
                seen: vec![5, 1, 5, 3],
            },
        );
        let (never, _) = b.input::<Vec<u32>>();
        let numbers = never.hold(b, vec![4, 4, 2, 8]);
        (b.anchor(stats), b.anchor(numbers))
    });
    rt.mutate(|b| {
        let (stats, numbers) = (b.open(&stats), b.open(&numbers));
        b.update(stats, |mut p| {
            let s: &mut Stats = p.as_mut();
            s.seen.sort();
            s.seen.dedup();
            s.count = s.seen.len() as u32;
        })
        .expect("live");
        b.update(numbers, |mut p| {
            let mut list = p.as_mut();
            let v: &mut Vec<u32> = list.as_mut_vec();
            v.dedup();
            v.retain(|&n| n > 2);
        })
        .expect("live");
        let s: &Stats = b.sample_ref(stats).expect("live");
        println!("stats:   {s:?}, mean {}", mean(s));
        let n: &[u32] = b.sample_ref(numbers).expect("live").as_slice();
        println!("numbers: {n:?}, contains 8: {}", n.contains(&8));
    });
}
