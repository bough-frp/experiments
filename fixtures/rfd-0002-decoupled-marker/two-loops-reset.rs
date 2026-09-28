//! Two loops, one reading the other's steps: a block number, and a retry
//! count that resets whenever the block number steps. The retry count
//! depends on the block number this instant; the block number does not
//! depend on the retry count, so the graph is acyclic. Written the natural
//! way, through the forward reference.
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
    let reset = block.steps().map(|_| 0);
    let next_retry = bumped.merge(&mut b, reset).hold(&mut b, 0);
    retry_loop.close(&mut b, next_retry);
}
