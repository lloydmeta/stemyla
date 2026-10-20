//! Tiny sound helpers, written so a beginner can read every line.
//!
//! A sound is a long list of numbers between -1.0 and 1.0 called samples.
//! Each function here takes a list of samples and does one small job.

// Clippy suggests shortcuts that a beginner does not know yet.
// Each lint below fires on a form that DSP-1 keeps on purpose.
// All other lints keep their workspace level.  If a lint below
// stops firing, clippy warns, and `-D warnings` (as in CI) fails.
// Then remove that lint from the list.
#![expect(
    clippy::assign_op_pattern,
    reason = "DSP-1: `x = x + 1` and `x = x - 1` show the sum in full"
)]
#![expect(
    clippy::len_zero,
    reason = "DSP-1: `len() > 0` needs only `len`, not `is_empty`"
)]
#![expect(
    clippy::manual_clamp,
    reason = "DSP-1: an `if` for each bound shows what a clamp does"
)]
#![expect(clippy::manual_swap, reason = "DSP-1 forbids `swap`")]

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
