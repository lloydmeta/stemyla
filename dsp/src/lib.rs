//! Tiny sound helpers, written so a beginner can read every line.
//!
//! A sound is a long list of numbers between -1.0 and 1.0 called samples.
//! Each function here takes a list of samples and does one small job.

// Clippy would suggest iterators, closures and other shortcuts here.  This
// crate is deliberately written with plain `for`, `while`, `if` and indexing
// so that a 10-year-old can follow it, so all Clippy lints are switched off.
#![allow(clippy::all)]

mod biggest;
mod double_speed;
mod fade_in;
mod median;
mod mix;
mod peaks;
mod wrap_loop;

pub use biggest::biggest;
pub use double_speed::double_speed;
pub use fade_in::fade_in;
pub use median::median;
pub use mix::mix;
pub use peaks::peaks;
pub use wrap_loop::wrap_loop;
