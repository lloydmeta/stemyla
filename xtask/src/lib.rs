//! Offline tooling: synthetic song, metrics, benches, model conversion, the static server.

pub mod bench;
pub mod drivers;
pub mod metrics;
pub mod publish;
pub mod roformer_cli;
pub mod separate;
pub mod serve;
pub mod web_test;
/// The synthetic song lives in its own crate so the app can use it too.
pub use ::synth;
