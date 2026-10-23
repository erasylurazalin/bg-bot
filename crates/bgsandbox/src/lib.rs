//! A one-player terminal sandbox over bgcore: play shop phases against a
//! rigged Tavern, fight a copy of your own board each turn, and record every
//! end-of-turn board to `boards/`. See `Session`.

pub mod parse;
pub mod screen;
pub mod session;

pub use session::{Config, Reply, Session};
