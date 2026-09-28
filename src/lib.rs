//! The code under test of each probe whose benches share it with a binary
//! or with each other, one module per probe, named after it.

pub mod rfd_0003_sweep_cost;
pub mod rfd_0003_work_paced_trigger;
pub mod rfd_0004_erased_materializer;
pub mod rfd_0004_patch_cell_crossover;
pub mod rfd_0005_bounded_relink_check;
pub mod rfd_0005_cycle_in_mark;
pub mod rfd_0005_demand_bounded_push;
pub mod rfd_0005_heap_vs_mark_on_quiet_regions;
pub mod rfd_0005_maintained_rank_queue;
pub mod rfd_0005_small_side_order;
pub mod rfd_0006_lock_vs_queue_cost;
