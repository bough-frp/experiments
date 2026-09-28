//! `two-loops-reset` in the marker-close design: the block number's close
//! returns its cell re-marked, and the code after it shadows the forward
//! with that, so the retry count reads it as before. Same graph, same
//! text but the one `let`.
//@ legal yes
//@ designs baseline marker-close
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-close.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let blocks = b.input::<()>();
    let retries = b.input::<()>();
    let (block, block_loop) = b.cell_loop::<u64>();
    let (retry, retry_loop) = b.cell_loop::<u32>();

    let next_block = blocks.snapshot(block, |_, n| n + 1).hold(&mut b, 0);
    let block = block_loop.close(&mut b, next_block);

    let bumped = retries.snapshot(retry, |_, n| n + 1);
    let reset = block.steps().map(|_| 0);
    let next_retry = bumped.merge(&mut b, reset).hold(&mut b, 0);
    let _retry = retry_loop.close(&mut b, next_retry);
}
