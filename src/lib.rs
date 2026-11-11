//! Ez2Nec: convert EZNEC `.ez` antenna models to NEC decks.
//!
//! [`ez::read_ez`] decodes a file into an [`ez::EzModel`]; [`dump::dump`]
//! renders one as text for checking against EZNEC's own screens.

pub mod dump;
pub mod ez;
pub mod rules;
