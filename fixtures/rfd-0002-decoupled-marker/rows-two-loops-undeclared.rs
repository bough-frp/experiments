//! `two-loops-reset` under rows with the retry count's declaration not
//! listing the block number: legal as a graph, refused by the declaration.
//@ legal yes
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let blocks = b.input::<()>();
    let retries = b.input::<()>();
    let (block, block_loop) = b.cell_loop::<u64, L0, Empty>();
    let (retry, retry_loop) = b.cell_loop::<u32, L1, Empty>();

    let next_block = blocks.snapshot(block, |_, n| n + 1).hold(&mut b, 0);
    block_loop.close(&mut b, next_block);

    let bumped = retries.snapshot(retry, |_, n| n + 1);
    let reset = block.steps().map(|_| 0);
    let next_retry = bumped.merge(&mut b, reset).hold(&mut b, 0);
    retry_loop.close(&mut b, next_retry);
}
