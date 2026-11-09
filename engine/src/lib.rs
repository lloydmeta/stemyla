//! Stem explorer engine: decoding, stems, lane overviews, transport,
//! stretching and offline rendering.  Live audio output sits behind the
//! `cpal` feature.

mod aac_config;
mod adpcm;
mod adts_split;
pub mod decode;
mod hidden;
pub mod hpss;
mod ima4;
#[cfg(feature = "cpal")]
pub mod live;
pub mod mixdown;
mod mp4_boxes;
mod mp4_stream;
pub mod overview;
pub mod params;
pub mod player;
pub mod readahead;
pub mod resample;
pub mod slot;
pub mod song;
pub mod source;
pub mod stretch;
mod tag_stream;
#[cfg(test)]
mod test_alloc;
#[cfg(test)]
mod test_audio;
#[cfg(test)]
mod test_flac;
#[cfg(test)]
mod test_m4a;
pub mod wav;

pub use decode::{Audio, decode_bytes, decode_path};
pub use params::SharedParams;
pub use player::{Player, PlayerConfig};
pub use slot::PlayerSlot;
pub use song::{Samples, Song, Stem};
pub use stretch::{Stretcher, StretcherKind};
