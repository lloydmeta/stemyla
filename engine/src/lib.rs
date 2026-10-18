//! Stem explorer engine: decoding, stems, transport, stretching and
//! offline rendering.  Live audio output sits behind the `cpal` feature.

pub mod decode;
pub mod hpss;
#[cfg(feature = "cpal")]
pub mod live;
pub mod params;
pub mod player;
pub mod slot;
pub mod song;
pub mod stretch;
pub mod wav;

pub use decode::{Audio, decode_bytes, decode_path};
pub use params::SharedParams;
pub use player::{Player, PlayerConfig};
pub use slot::PlayerSlot;
pub use song::{Samples, Song, Stem};
pub use stretch::{Stretcher, StretcherKind};
