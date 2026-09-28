//! Instruction counts for the rebrand-cost probe: a thousand reads of a held
//! value (`read`), or a thousand events through a 22-node propagation that
//! reads it once each (`snapshot`, `switch_cell`, `sample`, `listener`), for
//! each value shape (`u64`, `three`, `vec10`, `vec1000`, `nested`) and each
//! way to read it: the `unsafe` cast gc-arena uses (`baseline`, an
//! experiment, not Bough code), a restored copy (`restore`), a borrow for a
//! brand-free type and a copy otherwise (`view`), and a derived borrowed view
//! (`borrow`). The module doc says what each is. Divide by a thousand for
//! instructions per read or per event.
//!
//! Each probe is built in setup and handed back for teardown, so neither its
//! build nor its drop is counted.

use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use bough_experiments::rfd_0003_rebrand_cost::{Op, Probe, Shape, Variant};

const N: u64 = 1000;

fn drop_probe(out: (u64, Probe)) {
    black_box(out.0);
}

fn baseline_probe(op: Op, shape: Shape) -> Probe {
    Probe::new(op, Variant::Baseline, shape)
}

fn restore_probe(op: Op, shape: Shape) -> Probe {
    Probe::new(op, Variant::Restore, shape)
}

fn view_probe(op: Op, shape: Shape) -> Probe {
    Probe::new(op, Variant::View, shape)
}

fn borrow_probe(op: Op, shape: Shape) -> Probe {
    Probe::new(op, Variant::Borrow, shape)
}

#[library_benchmark]
#[benches::read(args = [(Op::Read, Shape::U64), (Op::Read, Shape::Three), (Op::Read, Shape::Vec10), (Op::Read, Shape::Vec1000), (Op::Read, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
#[benches::snapshot(args = [(Op::Snapshot, Shape::U64), (Op::Snapshot, Shape::Three), (Op::Snapshot, Shape::Vec10), (Op::Snapshot, Shape::Vec1000), (Op::Snapshot, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
#[benches::switch_cell(args = [(Op::SwitchCell, Shape::U64), (Op::SwitchCell, Shape::Three), (Op::SwitchCell, Shape::Vec10), (Op::SwitchCell, Shape::Vec1000), (Op::SwitchCell, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
#[benches::sample(args = [(Op::Sample, Shape::U64), (Op::Sample, Shape::Three), (Op::Sample, Shape::Vec10), (Op::Sample, Shape::Vec1000), (Op::Sample, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
#[benches::listener(args = [(Op::Listener, Shape::U64), (Op::Listener, Shape::Three), (Op::Listener, Shape::Vec10), (Op::Listener, Shape::Vec1000), (Op::Listener, Shape::Nested)], setup = baseline_probe, teardown = drop_probe)]
fn baseline(mut probe: Probe) -> (u64, Probe) {
    (black_box(probe.run(N)), probe)
}

#[library_benchmark]
#[benches::read(args = [(Op::Read, Shape::U64), (Op::Read, Shape::Three), (Op::Read, Shape::Vec10), (Op::Read, Shape::Vec1000), (Op::Read, Shape::Nested)], setup = restore_probe, teardown = drop_probe)]
#[benches::snapshot(args = [(Op::Snapshot, Shape::U64), (Op::Snapshot, Shape::Three), (Op::Snapshot, Shape::Vec10), (Op::Snapshot, Shape::Vec1000), (Op::Snapshot, Shape::Nested)], setup = restore_probe, teardown = drop_probe)]
#[benches::switch_cell(args = [(Op::SwitchCell, Shape::U64), (Op::SwitchCell, Shape::Three), (Op::SwitchCell, Shape::Vec10), (Op::SwitchCell, Shape::Vec1000), (Op::SwitchCell, Shape::Nested)], setup = restore_probe, teardown = drop_probe)]
#[benches::sample(args = [(Op::Sample, Shape::U64), (Op::Sample, Shape::Three), (Op::Sample, Shape::Vec10), (Op::Sample, Shape::Vec1000), (Op::Sample, Shape::Nested)], setup = restore_probe, teardown = drop_probe)]
#[benches::listener(args = [(Op::Listener, Shape::U64), (Op::Listener, Shape::Three), (Op::Listener, Shape::Vec10), (Op::Listener, Shape::Vec1000), (Op::Listener, Shape::Nested)], setup = restore_probe, teardown = drop_probe)]
fn restore(mut probe: Probe) -> (u64, Probe) {
    (black_box(probe.run(N)), probe)
}

#[library_benchmark]
#[benches::read(args = [(Op::Read, Shape::U64), (Op::Read, Shape::Three), (Op::Read, Shape::Vec10), (Op::Read, Shape::Vec1000), (Op::Read, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
#[benches::snapshot(args = [(Op::Snapshot, Shape::U64), (Op::Snapshot, Shape::Three), (Op::Snapshot, Shape::Vec10), (Op::Snapshot, Shape::Vec1000), (Op::Snapshot, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
#[benches::switch_cell(args = [(Op::SwitchCell, Shape::U64), (Op::SwitchCell, Shape::Three), (Op::SwitchCell, Shape::Vec10), (Op::SwitchCell, Shape::Vec1000), (Op::SwitchCell, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
#[benches::sample(args = [(Op::Sample, Shape::U64), (Op::Sample, Shape::Three), (Op::Sample, Shape::Vec10), (Op::Sample, Shape::Vec1000), (Op::Sample, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
#[benches::listener(args = [(Op::Listener, Shape::U64), (Op::Listener, Shape::Three), (Op::Listener, Shape::Vec10), (Op::Listener, Shape::Vec1000), (Op::Listener, Shape::Nested)], setup = view_probe, teardown = drop_probe)]
fn view(mut probe: Probe) -> (u64, Probe) {
    (black_box(probe.run(N)), probe)
}

#[library_benchmark]
#[benches::read(args = [(Op::Read, Shape::U64), (Op::Read, Shape::Three), (Op::Read, Shape::Vec10), (Op::Read, Shape::Vec1000), (Op::Read, Shape::Nested)], setup = borrow_probe, teardown = drop_probe)]
#[benches::snapshot(args = [(Op::Snapshot, Shape::U64), (Op::Snapshot, Shape::Three), (Op::Snapshot, Shape::Vec10), (Op::Snapshot, Shape::Vec1000), (Op::Snapshot, Shape::Nested)], setup = borrow_probe, teardown = drop_probe)]
#[benches::switch_cell(args = [(Op::SwitchCell, Shape::U64), (Op::SwitchCell, Shape::Three), (Op::SwitchCell, Shape::Vec10), (Op::SwitchCell, Shape::Vec1000), (Op::SwitchCell, Shape::Nested)], setup = borrow_probe, teardown = drop_probe)]
#[benches::sample(args = [(Op::Sample, Shape::U64), (Op::Sample, Shape::Three), (Op::Sample, Shape::Vec10), (Op::Sample, Shape::Vec1000), (Op::Sample, Shape::Nested)], setup = borrow_probe, teardown = drop_probe)]
#[benches::listener(args = [(Op::Listener, Shape::U64), (Op::Listener, Shape::Three), (Op::Listener, Shape::Vec10), (Op::Listener, Shape::Vec1000), (Op::Listener, Shape::Nested)], setup = borrow_probe, teardown = drop_probe)]
fn borrow(mut probe: Probe) -> (u64, Probe) {
    (black_box(probe.run(N)), probe)
}

library_benchmark_group!(
    name = rebrand;
    benchmarks = baseline, restore, view, borrow
);

main!(library_benchmark_groups = rebrand);
