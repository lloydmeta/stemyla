//! Offline tooling: synthetic song, metrics, benches, model conversion, the static server.

pub mod bench;
pub mod metrics;
pub mod roformer_cli;
pub mod separate;
pub mod publish;
pub mod serve;
/// The synthetic song lives in its own crate so the app can use it too.
pub use ::synth;
