//! Another crate's declarative macro that writes an `unsafe impl`, for
//! `unsafe-macro.rs`: any crate's, not Bough's.
#![allow(unsafe_code)]

#[macro_export]
macro_rules! unsafe_impl {
    ($t:path, $ty:ty) => {
        unsafe impl $t for $ty {}
    };
}
