//! Writing NEC decks from a decoded model.

mod complex;
mod elements;
mod write;

pub use write::{ConvertError, Deck, Engine, Options, write_deck};

#[cfg(test)]
mod tests;
