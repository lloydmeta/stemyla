use anyhow::{Result, bail};
use std::path::PathBuf;
use xtask::{bench, e2e, publish, roformer_cli, separate, serve, synth, web_test};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        bail!(
            "usage: xtask <serve|web-test|e2e|publish|roformer-convert|roformer-selftest|roformer|gen-song|stretch-bench|hpss|sdr|classic|perf> [args]"
        );
    };
    match cmd.as_str() {
        "gen-song" => {
            let dir = args
                .get(1)
                .map_or_else(|| PathBuf::from("audio/synth"), PathBuf::from);
            let song = synth::generate();
            let has_ffmpeg = std::process::Command::new("ffmpeg")
                .arg("-version")
                .output()
                .is_ok();
            synth::write_song(&song, &dir, has_ffmpeg)?;
            println!(
                "wrote {} stems + mix + truth.json to {} ({} frames, normalisation {:.4}, mp3: {})",
                song.stems.len(),
                dir.display(),
                song.truth.frames,
                song.truth.normalisation,
                has_ffmpeg
            );
        }
        "stretch-bench" => bench::run(&args[1..])?,
        "perf" => bench::perf(&args[1..])?,
        "live-smoke" => live_smoke()?,
        "hpss" | "sdr" | "classic" => separate::run(cmd, &args[1..])?,
        "roformer" => roformer_cli::run(&args[1..])?,
        "roformer-convert" => roformer_cli::convert(&args[1..])?,
        "roformer-selftest" => roformer_cli::selftest_fixtures(&args[1..])?,
        "serve" => serve::run(&args[1..])?,
        "web-test" => web_test::run(&args[1..])?,
        "e2e" => e2e::run(&args[1..])?,
        "publish" => publish::main(&args[1..])?,
        other => bail!("unknown command {other}"),
    }
    Ok(())
}

/// M5: play two seconds of the synthetic song on the default output device
/// and report callback timing, or explain why there is no device.
#[cfg(feature = "cpal")]
fn live_smoke() -> Result<()> {
    use engine::{Player, SharedParams, StretcherKind};
    use std::sync::Arc;
    use std::sync::atomic::Ordering::Relaxed;
    match engine::live::probe_default_output() {
        Err(e) => {
            println!("NO DEVICE: {e:#}");
            return Ok(());
        }
        Ok(d) => println!("device: {d}"),
    }
    let (song, _) = bench::perf_song(6);
    let params = Arc::new(SharedParams::new(song.stems.len()));
    params.set_tempo(0.75);
    params.playing.store(true, Relaxed);
    let player = Player::new(
        song.clone(),
        params.clone(),
        StretcherKind::default_pitch_preserving(),
    );
    let out = engine::live::start(player).map_err(|(e, _)| e)?;
    std::thread::sleep(std::time::Duration::from_secs(2));
    let n = params.callbacks.load(Relaxed);
    let max_ns = params.max_callback_ns.load(Relaxed);
    println!(
        "played 2 s on {} at {} Hz, {} channels: {n} callbacks, max engine time {:.3} ms, playhead {}",
        out.device_name,
        out.sample_rate,
        out.channels,
        max_ns as f64 / 1e6,
        params.playhead()
    );
    if n == 0 {
        bail!("no callbacks arrived");
    }
    Ok(())
}

#[cfg(not(feature = "cpal"))]
fn live_smoke() -> Result<()> {
    bail!("build with --features cpal")
}
