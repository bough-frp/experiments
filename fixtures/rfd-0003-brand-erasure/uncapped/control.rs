//! The `uncapped` mode's control: `unsafe` under `#![forbid(unsafe_code)]`,
//! which must fail uncapped. Capped, it builds: the cap is what hid it.
#![forbid(unsafe_code)]

fn main() {
    let x = 7u32;
    // Sound, and still `unsafe_code`: the lint is about the keyword.
    let y = unsafe { *std::ptr::addr_of!(x) };
    println!("{y}");
}
