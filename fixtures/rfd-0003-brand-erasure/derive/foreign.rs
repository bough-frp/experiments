//! A third crate, neither Bough nor its user: a type a user would want in a
//! stream or a stored struct, which implements no Bough trait.
#[derive(Clone, Debug, PartialEq)]
pub struct Celsius(pub f64);
