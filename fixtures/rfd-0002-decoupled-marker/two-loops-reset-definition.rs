//! `two-loops-reset` with the retry count reading the block number's
//! definition, not its forward reference. Same graph; possible only when
//! the definition exists before the reader is built.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let blocks = b.input::<()>();
    let retries = b.input::<()>();
    let (block, block_loop) = b.cell_loop::<u64>();
    let (retry, retry_loop) = b.cell_loop::<u32>();

    let next_block = blocks.snapshot(block, |_, n| n + 1).hold(&mut b, 0);
    block_loop.close(&mut b, next_block);

    let bumped = retries.snapshot(retry, |_, n| n + 1);
    let reset = next_block.steps().map(|_| 0);
    let next_retry = bumped.merge(&mut b, reset).hold(&mut b, 0);
    retry_loop.close(&mut b, next_retry);
}
