//! `xtask classic`: HPSS on the synthetic song, SDR against ground truth.
//! `xtask hpss <in> <out_dir> [start_s] [dur_s]`: HPSS on any file.
//! `xtask sdr <truth_dir> <estimate_dir>`: SDR per stem of any separation.

use crate::metrics::sdr;
use crate::synth;
use anyhow::{Context, Result, bail};
use engine::hpss::{HpssConfig, hpss};
use std::path::Path;

pub fn run(cmd: &str, args: &[String]) -> Result<()> {
    match cmd {
        "classic" => classic(),
        "hpss" => {
            let [input, out, rest @ ..] = args else {
                bail!("usage: xtask hpss <in.wav|mp3> <out_dir> [start_s] [dur_s]");
            };
            let start: f64 = rest.first().map_or(Ok(0.0), |s| s.parse())?;
            let dur: Option<f64> = rest.get(1).map(|s| s.parse()).transpose()?;
            hpss_file(Path::new(input), Path::new(out), start, dur)
        }
        "sdr" => {
            let [truth, est] = args else {
                bail!("usage: xtask sdr <truth_dir> <estimate_dir>");
            };
            sdr_dirs(Path::new(truth), Path::new(est))
        }
        _ => bail!("unknown command {cmd}"),
    }
}

/// Stereo SDR: both channels concatenated.
pub fn sdr_stereo(r: (&[f32], &[f32]), e: (&[f32], &[f32])) -> f64 {
    let n = r.0.len().min(e.0.len());
    let rr: Vec<f32> = r.0[..n].iter().chain(&r.1[..n]).cloned().collect();
    let ee: Vec<f32> = e.0[..n].iter().chain(&e.1[..n]).cloned().collect();
    sdr(&rr, &ee)
}

pub struct ClassicResult {
    pub label: String,
    pub sdr_percussive: f64,
    pub sdr_mix: f64,
    pub sdr_harmonic: f64,
    pub sdr_mix_vs_harmonic_truth: f64,
    pub recon_err: f32,
}

pub fn classic_on_synth(cfg: &HpssConfig, label: &str) -> ClassicResult {
    let s = synth::generate();
    let drums = &s.stems.iter().find(|(n, _)| n == "drums").unwrap().1;
    let mix = &s.mix;
    let hl = hpss(&mix.left, cfg);
    let hr = hpss(&mix.right, cfg);
    let not_drums_l: Vec<f32> = mix
        .left
        .iter()
        .zip(&drums.left)
        .map(|(m, d)| m - d)
        .collect();
    let not_drums_r: Vec<f32> = mix
        .right
        .iter()
        .zip(&drums.right)
        .map(|(m, d)| m - d)
        .collect();
    let recon_err = (0..mix.left.len())
        .map(|i| {
            (hl.harmonic[i] + hl.percussive[i] + hl.residual.get(i).copied().unwrap_or(0.0)
                - mix.left[i])
                .abs()
        })
        .fold(0.0f32, f32::max);
    ClassicResult {
        label: label.into(),
        sdr_percussive: sdr_stereo(
            (&drums.left, &drums.right),
            (&hl.percussive, &hr.percussive),
        ),
        sdr_mix: sdr_stereo((&drums.left, &drums.right), (&mix.left, &mix.right)),
        sdr_harmonic: sdr_stereo((&not_drums_l, &not_drums_r), (&hl.harmonic, &hr.harmonic)),
        sdr_mix_vs_harmonic_truth: sdr_stereo(
            (&not_drums_l, &not_drums_r),
            (&mix.left, &mix.right),
        ),
        recon_err,
    }
}

fn classic() -> Result<()> {
    let variants = [
        (
            "default: 2048/512, 17x17, soft p=2, margin 1",
            HpssConfig::default(),
        ),
        (
            "31x31, soft p=2, margin 1",
            HpssConfig {
                harmonic_len: 31,
                percussive_len: 31,
                ..Default::default()
            },
        ),
        (
            "17x17, soft p=1, margin 1",
            HpssConfig {
                power: 1.0,
                ..Default::default()
            },
        ),
        (
            "17x17, binary, margin 2",
            HpssConfig {
                margin: 2.0,
                ..Default::default()
            },
        ),
        (
            "1024/256, 17x17, soft p=2",
            HpssConfig {
                n_fft: 1024,
                hop: 256,
                ..Default::default()
            },
        ),
    ];
    println!(
        "| Variant | SDR percussive vs true drums | SDR raw mix vs true drums | SDR harmonic vs true non-drums | SDR raw mix vs true non-drums | max |H+P(+R)−x| |"
    );
    println!("| - | - | - | - | - | - |");
    for (label, cfg) in variants {
        let t = std::time::Instant::now();
        let r = classic_on_synth(&cfg, label);
        println!(
            "| {} | {:.2} dB | {:.2} dB | {:.2} dB | {:.2} dB | {:.1e} |",
            r.label,
            r.sdr_percussive,
            r.sdr_mix,
            r.sdr_harmonic,
            r.sdr_mix_vs_harmonic_truth,
            r.recon_err
        );
        eprintln!("{label}: {:.1} s", t.elapsed().as_secs_f64());
    }
    Ok(())
}

pub fn write_wav(path: &Path, l: &[f32], r: &[f32], rate: u32) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for i in 0..l.len() {
        w.write_sample(l[i])?;
        w.write_sample(r[i])?;
    }
    w.finalize()?;
    Ok(())
}

fn hpss_file(input: &Path, out: &Path, start_s: f64, dur_s: Option<f64>) -> Result<()> {
    let a = engine::decode_path(input)?;
    let rate = a.sample_rate;
    let s = (start_s * rate as f64) as usize;
    let e = dur_s.map_or(a.frames(), |d| {
        (s + (d * rate as f64) as usize).min(a.frames())
    });
    let l = &a.channels[0][s..e];
    let r = &a.channels[a.channels.len().min(2) - 1][s..e];
    let t = std::time::Instant::now();
    let cfg = HpssConfig::default();
    let hl = hpss(l, &cfg);
    let hr = hpss(r, &cfg);
    let secs = t.elapsed().as_secs_f64();
    std::fs::create_dir_all(out)?;
    write_wav(&out.join("original.wav"), l, r, rate)?;
    write_wav(
        &out.join("percussive.wav"),
        &hl.percussive,
        &hr.percussive,
        rate,
    )?;
    write_wav(&out.join("harmonic.wav"), &hl.harmonic, &hr.harmonic, rate)?;
    let audio_s = (e - s) as f64 / rate as f64;
    println!(
        "HPSS of {} ({audio_s:.1} s at {rate} Hz, from {start_s} s): {secs:.1} s CPU (RTF {:.3}); wrote original/percussive/harmonic.wav to {}",
        input.display(),
        secs / audio_s,
        out.display()
    );
    Ok(())
}

fn sdr_dirs(truth: &Path, est: &Path) -> Result<()> {
    for name in synth::STEM_NAMES {
        let t = engine::decode_path(&truth.join(format!("{name}.wav")))
            .with_context(|| format!("truth {name}"))?;
        for cand in ["drums", "bass", "guitar", "piano", "other", "vocals"] {
            let p = est.join(format!("{cand}.wav"));
            if !p.exists() {
                continue;
            }
            let e = engine::decode_path(&p)?;
            let v = sdr_stereo(
                (&t.channels[0], &t.channels[1]),
                (&e.channels[0], &e.channels[1]),
            );
            println!("truth {name:6} vs estimate {cand:6}: {v:7.2} dB");
        }
    }
    Ok(())
}
