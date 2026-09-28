//! `two-loops-reset` with the retry count opened after the block number
//! closes, reading the block number's forward. The close left no loop
//! open, so the forward is from an old epoch. Legal; the marker refuses
//! the same order.
//@ legal yes
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let blocks = b.input::<()>();
    let retries = b.input::<()>();
    let (mut b, block, block_loop) = b.cell_loop::<u64>();
    let next_block = blocks.snapshot(block, |_, n| n + 1).hold(&mut b, 0);
    let (b, _block) = block_loop.close(b, next_block);

    let (mut b, retry, retry_loop) = b.cell_loop::<u32>();
    let bumped = retries.snapshot(retry, |_, n| n + 1);
    let reset = block.steps().map(|_| 0);
    let next_retry = bumped.merge(&mut b, reset).hold(&mut b, 0);
    let (_b, _retry) = retry_loop.close(b, next_retry);
}
