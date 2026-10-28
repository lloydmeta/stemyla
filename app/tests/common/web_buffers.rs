//! The buffers of the page's Web Audio output, for `web_output.rs`: how
//! many play in a time, and the band of buffers rendered that tells one of
//! each late pair from both.  `output.rs` checks them natively.

use engine::slot::WEB_BUFFER_FRAMES;
use std::ops::RangeInclusive;
use std::time::Duration;

/// The number of buffers of [`WEB_BUFFER_FRAMES`] frames that play in
/// `elapsed` at `rate` frames a second.
pub fn buffers_in(elapsed: Duration, rate: u32) -> f64 {
    elapsed.as_secs_f64() * f64::from(rate) / f64::from(WEB_BUFFER_FRAMES)
}

/// The numbers of buffers rendered in the time of `buffers` buffers when
/// one of each late pair is rendered: half to one and a half times
/// `buffers`.  Both of each pair (twice `buffers`) is outside the band.
pub fn rendered_once(buffers: f64) -> RangeInclusive<f64> {
    buffers / 2.0..=buffers * 1.5
}
