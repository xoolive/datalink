//! Public datalink wire contract types.
//!
//! The binary publishes [`DecodedEvent`] values over JSONL and Redis. Consumers
//! should depend on these Rust types.

pub mod event;

pub use acars;
pub use event::*;
