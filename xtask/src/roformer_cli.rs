//! `xtask roformer <weights.safetensors> <in.wav|mp3> <out_dir> [start_s] [dur_s]`:
//! six-stem separation with the Rust BS-RoFormer port on the CPU (Burn
//! ndarray backend).  Writes `<out_dir>/<stem>.wav` (32-bit float).
//!
//! `xtask roformer-convert <BS-Rofo-SW-Fixed.ckpt> <out.safetensors> [f32]`:
//! the checkpoint to the safetensors file the port and the web app load,
//! in Rust (Burn's PyTorch reader), f16 unless `f32` is given.  Same
//! tensors as `tools/roformer_export.py`.

use anyhow::{Context, Result, bail};
use burn::backend::NdArray;
use roformer::demix::Demixer;
use roformer::model::{BsRoformer, Options};
use roformer::separate::separate_chunk;
use roformer::{CHUNK, OVERLAP, SAMPLE_RATE, STEMS};
use std::path::Path;
use std::time::Instant;

use crate::separate::write_wav;

pub fn run(args: &[String]) -> Result<()> {
    let [weights, input, out, rest @ ..] = args else {
        bail!(
            "usage: xtask roformer <weights.safetensors> <in.wav|mp3> <out_dir> [start_s] [dur_s]"
        );
    };
    let start: f64 = rest.first().map_or(Ok(0.0), |s| s.parse())?;
    let dur: Option<f64> = rest.get(1).map(|s| s.parse()).transpose()?;
    let overlap: usize = std::env::var("ROFORMER_OVERLAP").map_or(Ok(OVERLAP), |v| v.parse())?;

    let audio = engine::decode_path(Path::new(input))?;
    if audio.sample_rate != SAMPLE_RATE {
        bail!(
            "{} Hz input; the model needs {SAMPLE_RATE} Hz",
            audio.sample_rate
        );
    }
    let ch = |c: usize| audio.channels[c.min(audio.channels.len() - 1)].clone();
    let from = (start * SAMPLE_RATE as f64) as usize;
    let to = dur.map_or(audio.frames(), |d| {
        (from + (d * SAMPLE_RATE as f64) as usize).min(audio.frames())
    });
    let mix = [ch(0)[from..to].to_vec(), ch(1)[from..to].to_vec()];

    let t = Instant::now();
    let bytes = std::fs::read(weights).with_context(|| format!("reading {weights}"))?;
    let model = BsRoformer::<NdArray>::load(&bytes, &Default::default(), Options::default())?;
    drop(bytes);
    eprintln!("weights loaded in {:.1} s", t.elapsed().as_secs_f64());

    let t = Instant::now();
    let mut d = Demixer::new(mix, CHUNK, overlap, STEMS.len());
    let total = d.num_chunks();
    while let Ok(Some(x)) = d.next_input() {
        let tc = Instant::now();
        let i = d.chunk_index();
        let y = separate_chunk(&model, &x);
        d.accept(&y);
        eprintln!(
            "chunk {}/{total}: {:.1} s",
            i + 1,
            tc.elapsed().as_secs_f64()
        );
    }
    let stems = d.finish();
    let secs = t.elapsed().as_secs_f64();
    let dur_s = (to - from) as f64 / SAMPLE_RATE as f64;
    std::fs::create_dir_all(out)?;
    for (name, s) in STEMS.iter().zip(&stems) {
        write_wav(
            &Path::new(out).join(format!("{name}.wav")),
            &s[0],
            &s[1],
            SAMPLE_RATE,
        )?;
    }
    println!(
        "separated {dur_s:.1} s in {secs:.0} s (RTF {:.1}), overlap {overlap}",
        secs / dur_s
    );
    Ok(())
}

pub fn convert(args: &[String]) -> Result<()> {
    use burn::tensor::DType;
    use safetensors::{Dtype, tensor::TensorView};

    let [ckpt, out, rest @ ..] = args else {
        bail!("usage: xtask roformer-convert <checkpoint.ckpt> <out.safetensors> [f32]");
    };
    let f32_out = rest.first().is_some_and(|a| a == "f32");
    let reader =
        burn_store::pytorch::PytorchReader::new(ckpt).with_context(|| format!("reading {ckpt}"))?;
    let mut names: Vec<String> = reader.keys();
    names.sort();
    // The rotary frequencies are recomputed by the model; check, then drop.
    let expected: Vec<f32> = (0..32)
        .map(|i| 1.0 / 10000f32.powf((2 * i) as f32 / 64.0))
        .collect();
    let mut bufs: Vec<(String, Vec<usize>, Vec<u8>)> = Vec::new();
    let mut params = 0usize;
    for name in names {
        let snap = reader.get(&name).expect("listed key");
        let data = snap
            .to_data()
            .map_err(|e| anyhow::anyhow!("{name}: {e:?}"))?;
        if data.dtype != DType::F32 {
            bail!("{name}: expected f32, found {:?}", data.dtype);
        }
        let shape = data.shape.clone();
        let v: Vec<f32> = data
            .to_vec()
            .map_err(|e| anyhow::anyhow!("{name}: {e:?}"))?;
        if name.ends_with("rotary_embed.freqs") {
            let err = v
                .iter()
                .zip(&expected)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            if err > 1e-6 {
                bail!("{name} differs from the default rotary frequencies by {err}");
            }
            continue;
        }
        params += v.len();
        let bytes: Vec<u8> = if f32_out {
            v.iter().flat_map(|x| x.to_le_bytes()).collect()
        } else {
            v.iter()
                .flat_map(|x| half::f16::from_f32(*x).to_le_bytes())
                .collect()
        };
        bufs.push((name, shape, bytes));
    }
    let dtype = if f32_out { Dtype::F32 } else { Dtype::F16 };
    let views: Vec<(String, TensorView)> = bufs
        .iter()
        .map(|(n, s, b)| Ok((n.clone(), TensorView::new(dtype, s.clone(), b)?)))
        .collect::<Result<_, safetensors::SafeTensorError>>()
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let meta: std::collections::HashMap<String, String> = [
        ("model", "BS-RoFormer SW (BS-Rofo-SW-Fixed)"),
        ("instruments", "bass,drums,other,vocals,guitar,piano"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    safetensors::serialize_to_file(views, Some(meta), Path::new(out))
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    println!(
        "wrote {out}: {} tensors, {params} parameters, {}",
        bufs.len(),
        if f32_out { "f32" } else { "f16" }
    );
    Ok(())
}

/// `xtask roformer-selftest <weights.safetensors> <out_dir>`: reference data
/// for the in-browser self-test (`?selftest` in the web app), computed by
/// this crate's CPU port, which matches PyTorch (roformer/tests).  Input is
/// the synthetic song, regenerated here, so the files hold nothing
/// copyrighted and nothing from the checkpoint beyond activations.
///
/// * `stages.safetensors`: 1 s of features and the activations after the
///   band split, layers 0, 1, 9, 10, 11, the final norm and the six masks
///   (f16, model layout `[bands, frames, dim]`).
/// * `chunk.safetensors`: one full 588,800-sample chunk (f32) and the six
///   separated stems (f16).
pub fn selftest_fixtures(args: &[String]) -> Result<()> {
    use roformer::separate::{analyse, to_tensor};
    use safetensors::{Dtype, tensor::TensorView};

    let [weights, out] = args else {
        bail!("usage: xtask roformer-selftest <weights.safetensors> <out_dir>");
    };
    let song = synth::generate();
    let n = song.stems[0].1.left.len();
    let mut mix = [vec![0.0f32; n], vec![0.0f32; n]];
    for (_, s) in &song.stems {
        for i in 0..n {
            mix[0][i] += s.left[i];
            mix[1][i] += s.right[i];
        }
    }
    let sr = SAMPLE_RATE as usize;
    let bytes = std::fs::read(weights).with_context(|| format!("reading {weights}"))?;
    let dev = Default::default();
    let model = BsRoformer::<NdArray>::load(&bytes, &dev, Options::default())?;
    drop(bytes);

    let f16 = |v: Vec<f32>| -> Vec<u8> {
        v.iter()
            .flat_map(|x| half::f16::from_f32(*x).to_le_bytes())
            .collect()
    };
    let f32b = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let read3 = |t: burn::tensor::Tensor<NdArray, 3>| -> (Vec<usize>, Vec<f32>) {
        let d = t.into_data();
        (d.shape.clone(), d.to_vec().expect("f32"))
    };
    let write = |path: &Path, items: Vec<(String, Dtype, Vec<usize>, Vec<u8>)>| -> Result<()> {
        let views: Vec<(String, TensorView)> = items
            .iter()
            .map(|(n, d, s, b)| Ok((n.clone(), TensorView::new(*d, s.clone(), b)?)))
            .collect::<Result<_, safetensors::SafeTensorError>>()
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        safetensors::serialize_to_file(views, None, path).map_err(|e| anyhow::anyhow!("{e:?}"))
    };
    std::fs::create_dir_all(out)?;

    // Stages, 1 s from 10 s.
    let t = Instant::now();
    let clip = [
        mix[0][10 * sr..11 * sr].to_vec(),
        mix[1][10 * sr..11 * sr].to_vec(),
    ];
    let (_, feats) = analyse(&clip);
    let frames = feats.len() / roformer::FEATURES;
    let mut items = vec![(
        "features".to_string(),
        Dtype::F32,
        vec![frames, roformer::FEATURES],
        f32b(&feats),
    )];
    let mut x = model.band_split(to_tensor::<NdArray>(feats, &dev));
    let (s, v) = read3(x.clone());
    items.push(("band_split".into(), Dtype::F16, s, f16(v)));
    for i in 0..roformer::DEPTH {
        x = model.layer(i, x);
        if matches!(i, 0 | 1 | 9 | 10 | 11) {
            let (s, v) = read3(x.clone());
            items.push((format!("layer{i}"), Dtype::F16, s, f16(v)));
        }
    }
    let x = model.final_norm(x);
    let (s, v) = read3(x.clone());
    items.push(("final_norm".into(), Dtype::F16, s, f16(v)));
    for st in 0..STEMS.len() {
        let d = model.mask_one(st, x.clone()).into_data();
        let s = d.shape.clone();
        items.push((
            format!("mask{st}"),
            Dtype::F16,
            s,
            f16(d.to_vec().expect("f32")),
        ));
    }
    write(&Path::new(out).join("stages.safetensors"), items)?;
    eprintln!(
        "stages ({frames} frames) in {:.1} s",
        t.elapsed().as_secs_f64()
    );

    // One full chunk from 5 s.
    let t = Instant::now();
    let chunk = [
        mix[0][5 * sr..5 * sr + CHUNK].to_vec(),
        mix[1][5 * sr..5 * sr + CHUNK].to_vec(),
    ];
    let stems = separate_chunk(&model, &chunk);
    let mut audio = chunk[0].clone();
    audio.extend_from_slice(&chunk[1]);
    let mut all = Vec::with_capacity(STEMS.len() * 2 * CHUNK);
    for s in &stems {
        all.extend_from_slice(&s[0]);
        all.extend_from_slice(&s[1]);
    }
    write(
        &Path::new(out).join("chunk.safetensors"),
        vec![
            ("audio".into(), Dtype::F32, vec![2, CHUNK], f32b(&audio)),
            (
                "stems".into(),
                Dtype::F16,
                vec![STEMS.len(), 2, CHUNK],
                f16(all),
            ),
        ],
    )?;
    eprintln!("chunk in {:.1} s", t.elapsed().as_secs_f64());
    println!("wrote {out}/stages.safetensors and {out}/chunk.safetensors");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Finding 1-35 of review round 1 of M4: `xtask roformer` refuses what
    /// it cannot separate, before it loads the model.  The separation
    /// itself needs the weights, which CI does not have.
    #[test]
    fn roformer_refuses_bad_input_before_the_model() {
        let dir = std::env::temp_dir().join(format!("xtask-roformer-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = |name: &str, rate: u32| {
            let path = dir.join(name);
            write_wav(&path, &[0.0; 64], &[0.0; 64], rate).unwrap();
            path.to_str().unwrap().to_string()
        };
        let (at_48, at_44) = (wav("48k.wav", 48_000), wav("44k.wav", SAMPLE_RATE));
        let missing = dir.join("no-such-model").to_str().unwrap().to_string();
        let out = dir.join("out").to_str().unwrap().to_string();
        let error = |list: &[&str]| format!("{:#}", run(&args(list)).unwrap_err());

        let usage = error(&["only-the-weights"]);
        let rate = error(&[&missing, &at_48, &out]);
        let weights = error(&[&missing, &at_44, &out]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(usage.starts_with("usage: xtask roformer"), "{usage}");
        assert!(
            rate.contains("48000 Hz input; the model needs 44100 Hz"),
            "{rate}"
        );
        assert!(
            weights.contains("reading ") && weights.contains("no-such-model"),
            "{weights}"
        );
    }
}
