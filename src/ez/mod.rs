//! Reading EZNEC `.ez` model files.

mod bytes;
mod model;
mod read;

pub use bytes::{EzError, Unclaimed, UnknownField};
pub use model::*;
pub use read::{FORMAT_BYTE, GAP_LEN, HEADER_LEN, RECORD_LEN, read_ez};

#[cfg(test)]
mod tests;
