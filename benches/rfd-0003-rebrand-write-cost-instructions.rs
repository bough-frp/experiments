//! Instruction counts for the rebrand write-cost probe: a thousand writes
//! of a held value (`store`, a value made and stored; `update`,
//! `accumulate_mut`'s commit step), or a thousand events through a 22-node
//! propagation that writes it once each (`hold`, `accumulate`,
//! `switch_cell`), for each value shape (`u64`, `three`, `vec10`, `vec1000`,
//! `nested`) and each way to write it: the `unsafe` cast gc-arena uses
//! (`baseline`, an experiment, not Bough code), the brand-erasure fixture's
//! rebrand from `&T` (`rebrand`), a move for a brand-free type and a rebrand
//! otherwise (`view`), an owned rebrand that keeps a `Vec`'s buffer
//! (`owned`), and, on updates only, a derived mutable view (`borrow`). The
//! module doc says what each is. Divide by a thousand for instructions per
//! write or per event.
//!
//! Each probe is built in setup and handed back for teardown, so neither its
//! build nor its drop is counted.

use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use bough_experiments::rfd_0003_rebrand_write_cost::{Op, Probe, Shape, Variant};

const N: u64 = 1000;

fn drop_probe(out: (u64, Probe)) {
    black_box(out.0);
}

fn baseline_probe(op: Op, shape: Shape) -> Probe {
    Probe::new(op, Variant::Baseline, shape)
}

fn rebrand_probe(op: Op, shape: Shape) -> Probe {
    Probe::new(op, Variant::Rebrand, shape)
}

fn view_probe(op: Op, shape: Shape) -> Probe {
    Probe::new(op, Variant::View, shape)
}

fn owned_probe(op: Op, shape: Shape) -> Probe {
    Probe::new(op, Variant::Owned, shape)
}

fn borrow_probe(op: Op, shape: Shape) -> Probe {
    Probe::new(op, Variant::Borrow, shape)
}

#[library_benchmark]
#[benches::store(args = [(Op::Store, Shape::U64), (Op::Store, Shape::Three), (Op::Store, Shape::Vec10), (Op::Store, Shape::Vec1000), (Op::Store, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
#[benches::update(args = [(Op::Update, Shape::U64), (Op::Update, Shape::Three), (Op::Update, Shape::Vec10), (Op::Update, Shape::Vec1000), (Op::Update, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
#[benches::hold(args = [(Op::Hold, Shape::U64), (Op::Hold, Shape::Three), (Op::Hold, Shape::Vec10), (Op::Hold, Shape::Vec1000), (Op::Hold, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
#[benches::accumulate(args = [(Op::Accumulate, Shape::U64), (Op::Accumulate, Shape::Three), (Op::Accumulate, Shape::Vec10), (Op::Accumulate, Shape::Vec1000), (Op::Accumulate, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
#[benches::switch_cell(args = [(Op::SwitchCell, Shape::U64), (Op::SwitchCell, Shape::Three), (Op::SwitchCell, Shape::Vec10), (Op::SwitchCell, Shape::Vec1000), (Op::SwitchCell, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
fn baseline(mut probe: Probe) -> (u64, Probe) {
    (black_box(probe.run(N)), probe)
}

#[library_benchmark]
#[benches::store(args = [(Op::Store, Shape::U64), (Op::Store, Shape::Three), (Op::Store, Shape::Vec10), (Op::Store, Shape::Vec1000), (Op::Store, Shape::Nested)], setup = rebrand_probe, teardown = drop_probe)]
#[benches::update(args = [(Op::Update, Shape::U64), (Op::Update, Shape::Three), (Op::Update, Shape::Vec10), (Op::Update, Shape::Vec1000), (Op::Update, Shape::Nested)], setup = rebrand_probe, teardown = drop_probe)]
#[benches::hold(args = [(Op::Hold, Shape::U64), (Op::Hold, Shape::Three), (Op::Hold, Shape::Vec10), (Op::Hold, Shape::Vec1000), (Op::Hold, Shape::Nested)], setup = rebrand_probe, teardown = drop_probe)]
#[benches::accumulate(args = [(Op::Accumulate, Shape::U64), (Op::Accumulate, Shape::Three), (Op::Accumulate, Shape::Vec10), (Op::Accumulate, Shape::Vec1000), (Op::Accumulate, Shape::Nested)], setup = rebrand_probe, teardown = drop_probe)]
#[benches::switch_cell(args = [(Op::SwitchCell, Shape::U64), (Op::SwitchCell, Shape::Three), (Op::SwitchCell, Shape::Vec10), (Op::SwitchCell, Shape::Vec1000), (Op::SwitchCell, Shape::Nested)], setup = rebrand_probe, teardown = drop_probe)]
fn rebrand(mut probe: Probe) -> (u64, Probe) {
    (black_box(probe.run(N)), probe)
}

#[library_benchmark]
#[benches::store(args = [(Op::Store, Shape::U64), (Op::Store, Shape::Three), (Op::Store, Shape::Vec10), (Op::Store, Shape::Vec1000), (Op::Store, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
#[benches::update(args = [(Op::Update, Shape::U64), (Op::Update, Shape::Three), (Op::Update, Shape::Vec10), (Op::Update, Shape::Vec1000), (Op::Update, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
#[benches::hold(args = [(Op::Hold, Shape::U64), (Op::Hold, Shape::Three), (Op::Hold, Shape::Vec10), (Op::Hold, Shape::Vec1000), (Op::Hold, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
#[benches::accumulate(args = [(Op::Accumulate, Shape::U64), (Op::Accumulate, Shape::Three), (Op::Accumulate, Shape::Vec10), (Op::Accumulate, Shape::Vec1000), (Op::Accumulate, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
#[benches::switch_cell(args = [(Op::SwitchCell, Shape::U64), (Op::SwitchCell, Shape::Three), (Op::SwitchCell, Shape::Vec10), (Op::SwitchCell, Shape::Vec1000), (Op::SwitchCell, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
fn view(mut probe: Probe) -> (u64, Probe) {
    (black_box(probe.run(N)), probe)
}

#[library_benchmark]
#[benches::store(args = [(Op::Store, Shape::U64), (Op::Store, Shape::Three), (Op::Store, Shape::Vec10), (Op::Store, Shape::Vec1000), (Op::Store, Shape::Nested)], setup = owned_probe, teardown = drop_probe)]
#[benches::update(args = [(Op::Update, Shape::U64), (Op::Update, Shape::Three), (Op::Update, Shape::Vec10), (Op::Update, Shape::Vec1000), (Op::Update, Shape::Nested)], setup = owned_probe, teardown = drop_probe)]
#[benches::hold(args = [(Op::Hold, Shape::U64), (Op::Hold, Shape::Three), (Op::Hold, Shape::Vec10), (Op::Hold, Shape::Vec1000), (Op::Hold, Shape::Nested)], setup = owned_probe, teardown = drop_probe)]
#[benches::accumulate(args = [(Op::Accumulate, Shape::U64), (Op::Accumulate, Shape::Three), (Op::Accumulate, Shape::Vec10), (Op::Accumulate, Shape::Vec1000), (Op::Accumulate, Shape::Nested)], setup = owned_probe, teardown = drop_probe)]
#[benches::switch_cell(args = [(Op::SwitchCell, Shape::U64), (Op::SwitchCell, Shape::Three), (Op::SwitchCell, Shape::Vec10), (Op::SwitchCell, Shape::Vec1000), (Op::SwitchCell, Shape::Nested)], setup = owned_probe, teardown = drop_probe)]
fn owned(mut probe: Probe) -> (u64, Probe) {
    (black_box(probe.run(N)), probe)
}

#[library_benchmark]
#[benches::update(args = [(Op::Update, Shape::U64), (Op::Update, Shape::Three), (Op::Update, Shape::Vec10), (Op::Update, Shape::Vec1000), (Op::Update, Shape::Nested)], setup = borrow_probe, teardown = drop_probe)]
#[benches::accumulate(args = [(Op::Accumulate, Shape::U64), (Op::Accumulate, Shape::Three), (Op::Accumulate, Shape::Vec10), (Op::Accumulate, Shape::Vec1000), (Op::Accumulate, Shape::Nested)], setup = borrow_probe, teardown = drop_probe)]
fn borrow(mut probe: Probe) -> (u64, Probe) {
    (black_box(probe.run(N)), probe)
}

library_benchmark_group!(
    name = rebrand_write;
    benchmarks = baseline, rebrand, view, owned, borrow
);

main!(library_benchmark_groups = rebrand_write);
