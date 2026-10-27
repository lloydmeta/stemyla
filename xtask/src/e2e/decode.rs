//! `xtask e2e decode`: the decode worker in the built page (M5 of
//! `docs/plans/active/stream-from-storage.md`, step 5 of
//! `docs/plans/active/stream-from-storage-m5.md`).  No model and no GPU.
//!
//! It drops a 44.1 kHz WAV of one source, a 48 kHz WAV of one source and
//! the synthetic stems, and checks each session's files against the native
//! `engine` decode of the same bytes.  It reloads to restore the last song
//! with no decode, as stored lanes, then plants a session of the M4 layout (no `decoded`
//! line) and reloads to restore it through the decode worker.  Every case
//! fails on a line `decoder: decoding on the page`, and no `.part` file may
//! be left.

use super::{Checks, Opts, Page, in_page, is_page_error};
use anyhow::Result;
use engine::overview::lane_columns;
use engine::song::{Song, is_stem, rank, stem_name};
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;

/// The names that stay out of the mix, as `app::DERIVED` gives them
/// (`xtask` does not take `app`).
const DERIVED: [&str; 2] = ["classic drums", "original"];

/// The page's line when it decodes a song itself (D9, D11).
const ON_THE_PAGE: &str = "decoder: decoding on the page";

/// The page's line when it restores a complete session with no decode: as
/// stored lanes, with no read-back (step 7 of the M7 plan).
const STORED_OPEN: &str = "decoder: a complete session opens as stored lanes";

/// The decode worker's line as a decode starts.
const DECODE_START: &str = "at the decode's start";

/// The session files of a song as the decode worker writes them, from the
/// native decode of the same files.
#[derive(Debug, Default)]
pub(super) struct Expected {
    pub frames: usize,
    /// The SHA-256 of each file of samples, by its path from the session.
    pub hashes: BTreeMap<String, String>,
    /// The bytes of each overview, by its path from the session.
    pub overviews: BTreeMap<String, Vec<u8>>,
}

/// What the decode worker writes for the dropped `files` (name, bytes), in
/// the order dropped, as `engine` decodes them natively (D1).
pub(super) fn expected(files: &[(String, Vec<u8>)]) -> Result<Expected> {
    // Each source: its index in the drop (`src/<i>.<ext>`), its name, and
    // whether it is in the mix, in the order of `Song::from_audio`.
    let mut sources = Vec::new();
    let mut audio = Vec::new();
    for (i, (file, bytes)) in files.iter().enumerate() {
        let name = stem_name(file);
        if !is_stem(&name) {
            continue;
        }
        let ext = file.rsplit('.').next().map(str::to_ascii_lowercase);
        audio.push((
            name.clone(),
            engine::decode_bytes(bytes.clone(), ext.as_deref())?,
        ));
        sources.push((i, !DERIVED.contains(&name.as_str()), name));
    }
    sources.sort_by_key(|(_, _, name)| rank(name));
    let song = Song::from_audio(audio)?;
    let one = sources.iter().filter(|(_, in_mix, _)| *in_mix).count() == 1;
    let mut want = Expected {
        frames: song.frames,
        ..Expected::default()
    };
    for ((i, in_mix, _), stem) in sources.iter().zip(&song.stems) {
        let path = if one && *in_mix {
            "mix.f32".to_string()
        } else {
            format!("decoded/{i}.f32")
        };
        let hash = super::sha256_hex(&interleaved(&stem.left, &stem.right));
        want.hashes.insert(path, hash);
        let (cols, _) = lane_columns(&stem.left, &stem.right);
        want.overviews
            .insert(format!("overviews/{i}.f32"), column_bytes(&cols));
    }
    let in_mix: Vec<&engine::Stem> = sources
        .iter()
        .zip(&song.stems)
        .filter(|((_, in_mix, _), _)| *in_mix)
        .map(|(_, stem)| stem)
        .collect();
    let (mix_left, mix_right) = if in_mix.is_empty() {
        (vec![0.0; song.frames], vec![0.0; song.frames])
    } else {
        let ones = vec![1.0; in_mix.len()];
        let lefts: Vec<&[f32]> = in_mix.iter().map(|s| s.left.as_slice()).collect();
        let rights: Vec<&[f32]> = in_mix.iter().map(|s| s.right.as_slice()).collect();
        (dsp::mix(&lefts, &ones), dsp::mix(&rights, &ones))
    };
    if !one {
        let hash = super::sha256_hex(&interleaved(&mix_left, &mix_right));
        want.hashes.insert("mix.f32".into(), hash);
    }
    let (cols, _) = lane_columns(&mix_left, &mix_right);
    want.overviews
        .insert("overviews/mix.f32".into(), column_bytes(&cols));
    Ok(want)
}

/// Two channels as the bytes of a session file.
fn interleaved(left: &[f32], right: &[f32]) -> Vec<u8> {
    left.iter()
        .zip(right)
        .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
        .flatten()
        .collect()
}

/// Columns as the bytes of an overview file.
fn column_bytes(cols: &[(f32, f32)]) -> Vec<u8> {
    cols.iter()
        .flat_map(|(lo, hi)| [lo.to_le_bytes(), hi.to_le_bytes()])
        .flatten()
        .collect()
}

/// The checks on the session files that the page reads (`got`: `files`, a
/// list of `[path, size]` in the session folder, `hashes` and `overviews`
/// by path, and `meta`) against `want`.
pub(super) fn session_checks(got: &Value, want: &Expected) -> Checks {
    let mut checks = Vec::new();
    for (path, hash) in &want.hashes {
        let found = got["hashes"][path].as_str().unwrap_or("none");
        checks.push((
            found == hash,
            format!("{path} equals the native decode (SHA-256 {found}, want {hash})"),
        ));
    }
    for (path, bytes) in &want.overviews {
        let found = got["overviews"][path]
            .as_str()
            .and_then(|text| crate::webdriver::base64_decode(text).ok());
        checks.push((
            found.as_ref() == Some(bytes),
            format!("{path} equals engine::overview of the native decode"),
        ));
    }
    let extra: Vec<&str> = got["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f[0].as_str())
        .filter(|p| {
            *p != "meta"
                && !p.starts_with("src/")
                && !want.hashes.contains_key(*p)
                && !want.overviews.contains_key(*p)
        })
        .collect();
    checks.push((
        extra.is_empty(),
        format!("the session holds only the files of D1 (others: {extra:?})"),
    ));
    let meta = got["meta"].as_str().unwrap_or_default();
    let mix_line = format!("mix\t{}\tf32le-1", want.frames);
    let decoded_line = format!("decoded\t{}\tf32le-1\t", want.frames);
    checks.push((
        meta.lines().any(|l| l == mix_line),
        format!("meta has the line {mix_line:?}"),
    ));
    checks.push((
        meta.lines().any(|l| l.starts_with(&decoded_line)),
        format!("meta has a line that starts {decoded_line:?}"),
    ));
    checks
}

/// Whether `got` has the length of `want`, and each sample is within `tol`.
pub(super) fn close_check(got: &[f32], want: &[f32], tol: f32) -> (bool, String) {
    if got.len() != want.len() {
        return (
            false,
            format!(
                "{} frames, not the {} frames of the native decode",
                got.len(),
                want.len()
            ),
        );
    }
    let worst = got
        .iter()
        .zip(want)
        .map(|(g, w)| (g - w).abs())
        .fold(0.0_f32, f32::max);
    (
        worst <= tol,
        format!(
            "{} frames within {tol} of the native decode (worst {worst})",
            got.len()
        ),
    )
}

/// No path of `paths` (every file in the private file system) is a part
/// file.
pub(super) fn no_parts_check(paths: &[String]) -> (bool, String) {
    let parts: Vec<&String> = paths.iter().filter(|p| p.ends_with(".part")).collect();
    (
        parts.is_empty(),
        format!("no .part file is left ({parts:?})"),
    )
}

/// Every case fails on a line of the page's decode.
pub(super) fn never_on_the_page_check(log: &[String]) -> (bool, String) {
    let found: Vec<&String> = log.iter().filter(|l| l.contains(ON_THE_PAGE)).collect();
    (
        found.is_empty(),
        format!("no song is decoded on the page ({found:?})"),
    )
}

/// After a reload of a complete session: the page opened it as stored
/// lanes, and the decode worker did not start a decode.
pub(super) fn stored_open_checks(log: &[String]) -> Checks {
    vec![
        (
            log.iter().any(|l| l.contains(STORED_OPEN)),
            format!("the page logs {STORED_OPEN:?}"),
        ),
        (
            !log.iter().any(|l| l.contains(DECODE_START)),
            "the decode worker starts no decode".into(),
        ),
        never_on_the_page_check(log),
    ]
}

/// After a reload of a session of the M4 layout: the decode worker decoded
/// it, and the page restored it.
pub(super) fn decoded_again_checks(log: &[String]) -> Checks {
    vec![
        (
            log.iter().any(|l| l.contains(DECODE_START)),
            "the decode worker decodes the session".into(),
        ),
        (
            log.iter().any(|l| l.contains("Restored \"")),
            "the page restores the song".into(),
        ),
        never_on_the_page_check(log),
    ]
}

/// Reads the current session in the page: every file with its size, the
/// SHA-256 of each file of samples, each overview and `meta`, and with
/// `with_mix`, the bytes of `mix.f32` (base64).
const READ_SESSION: &str = r#"
const [withMix] = args;
const root = await navigator.storage.getDirectory();
const sessions = await root.getDirectoryHandle("sessions");
const id = (await (await (await sessions.getFileHandle("current")).getFile()).text()).trim();
const dir = await sessions.getDirectoryHandle(id);
const files = [], hashes = {}, overviews = {};
let meta = "", mix = null;
const hex = (b) => Array.from(new Uint8Array(b), (x) => x.toString(16).padStart(2, "0")).join("");
const b64 = (buf) => {
  const u = new Uint8Array(buf);
  let s = "";
  for (let i = 0; i < u.length; i += 0x8000) s += String.fromCharCode(...u.subarray(i, i + 0x8000));
  return btoa(s);
};
async function walk(d, at) {
  for await (const [name, h] of d.entries()) {
    const path = at + name;
    if (h.kind === "directory") { await walk(h, path + "/"); continue; }
    const f = await h.getFile();
    files.push([path, f.size]);
    if (path === "meta") meta = await f.text();
    else if (path.startsWith("overviews/")) overviews[path] = b64(await f.arrayBuffer());
    else if (path.endsWith(".f32")) {
      const buf = await f.arrayBuffer();
      hashes[path] = hex(await crypto.subtle.digest("SHA-256", buf));
      if (withMix && path === "mix.f32") mix = b64(buf);
    }
  }
}
await walk(dir, "");
return { id, files, hashes, overviews, meta, mix };
"#;

/// Every file in the page's private file system, by its path.
const ALL_FILES: &str = r#"
const out = [];
async function walk(d, at) {
  for await (const [name, h] of d.entries()) {
    if (h.kind === "directory") await walk(h, at + name + "/");
    else out.push(at + name);
  }
}
await walk(await navigator.storage.getDirectory(), "");
return out;
"#;

/// Plants a session of the M4 layout from `/decode/song.wav`: `meta` with
/// no `decoded` line, and the source, made current.
const PLANT_M4: &str = r#"
const [id] = args;
const root = await navigator.storage.getDirectory();
const sessions = await root.getDirectoryHandle("sessions", { create: true });
const dir = await sessions.getDirectoryHandle(id, { create: true });
const src = await dir.getDirectoryHandle("src", { create: true });
const put = async (d, name, body) => {
  const w = await (await d.getFileHandle(name, { create: true })).createWritable();
  await w.write(body);
  await w.close();
};
await put(src, "0.wav", await (await fetch("/decode/song.wav")).blob());
await put(dir, "meta", "name\tm4\nrate\t44100\nfile\tsong.wav\t0.wav\n");
await put(sessions, "current", id);
"#;

/// Drops `files` (served under `/decode/` or `/stems/`), waits for the
/// song to open from its session, and records the check.
fn drop_and_open(p: &mut Page, files: &[(&str, &str)], what: &str) -> Result<()> {
    let drop: Vec<(&str, &str, &str)> = files
        .iter()
        .map(|(url, name)| (*url, *name, "audio/wav"))
        .collect();
    let from = p.mark();
    p.drop_files(&drop)?;
    let kept = p.wait_for(from, 120, |l| {
        l.contains("song kept in browser storage as session") || is_page_error(l)
    })?;
    p.check(
        kept.as_deref()
            .is_some_and(|l| l.contains("song kept in browser storage as session")),
        &format!("{what} opens from its session files"),
    );
    Ok(())
}

fn check_all(p: &mut Page, checks: Checks) {
    for (ok, what) in checks {
        p.check(ok, &what);
    }
}

pub(super) fn run(o: &Opts) -> Result<()> {
    let song = crate::synth::generate();
    let n = 2 * crate::synth::SAMPLE_RATE as usize;
    let (left, right) = (&song.mix.left[..n], &song.mix.right[..n]);
    let dir = o.out.join("decode");
    std::fs::create_dir_all(&dir)?;
    let at_44 = engine::wav::encode_pcm16(left, right, 44_100);
    let at_48 = engine::wav::encode_pcm16(left, right, 48_000);
    std::fs::write(dir.join("song.wav"), &at_44)?;
    std::fs::write(dir.join("song48.wav"), &at_48)?;
    let stems = super::synth_stems(&o.out)?;
    let base = super::start_server(
        &o.dist,
        vec![("/decode/".into(), dir), ("/stems/".into(), stems.clone())],
        &["console.js"],
    )?;
    let mut p = Page::open(o, base, &[], "index.html")?;
    let ready = p.wait_for(0, 60, |l| l.contains("separator: "))?;
    p.check(ready.is_some(), "the app started");

    // A 44.1 kHz WAV of one source: its samples in mix.f32, bit for bit.
    drop_and_open(
        &mut p,
        &[("/decode/song.wav", "song.wav")],
        "a 44.1 kHz WAV",
    )?;
    let got = in_page(&mut p, READ_SESSION, json!([false]), 120)?;
    let want = expected(&[("song.wav".into(), at_44.clone())])?;
    check_all(&mut p, session_checks(&got, &want));

    // A 48 kHz WAV of one source: wasm and native `sin` may differ in the
    // last bit of the filter table, so the samples are within 1e-6.
    drop_and_open(
        &mut p,
        &[("/decode/song48.wav", "song48.wav")],
        "a 48 kHz WAV",
    )?;
    let got = in_page(&mut p, READ_SESSION, json!([true]), 120)?;
    let native = Song::from_audio(vec![(
        "song48".into(),
        engine::decode_bytes(at_48.clone(), Some("wav"))?,
    )])?;
    let mix = super::floats_from_base64(got["mix"].as_str().unwrap_or_default())?;
    let (got_left, got_right): (Vec<f32>, Vec<f32>) =
        mix.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
    let stem = &native.stems[0];
    for (side, got, want) in [
        ("left", &got_left, &stem.left),
        ("right", &got_right, &stem.right),
    ] {
        let (ok, what) = close_check(got, want, 1e-6);
        p.check(ok, &format!("the 48 kHz WAV's {side} side: {what}"));
    }
    let decoded = format!("decoded\t{}\tf32le-1\t", native.frames);
    p.check(
        got["meta"]
            .as_str()
            .is_some_and(|m| m.lines().any(|l| l.starts_with(&decoded))),
        &format!(
            "the 48 kHz WAV has the native decode's {} frames",
            native.frames
        ),
    );

    // The synthetic stems: a song of several sources.
    let names = ["drums", "bass", "guitar", "keys", "lead"];
    let files: Vec<(String, Vec<u8>)> = names
        .iter()
        .map(|n| {
            Ok((
                format!("{n}.wav"),
                std::fs::read(stems.join(format!("{n}.wav")))?,
            ))
        })
        .collect::<Result<_>>()?;
    let urls: Vec<(String, String)> = names
        .iter()
        .map(|n| (format!("/stems/{n}.wav"), format!("{n}.wav")))
        .collect();
    let drop: Vec<(&str, &str)> = urls.iter().map(|(u, f)| (u.as_str(), f.as_str())).collect();
    drop_and_open(&mut p, &drop, "the synthetic stems")?;
    let got = in_page(&mut p, READ_SESSION, json!([false]), 300)?;
    check_all(&mut p, session_checks(&got, &expected(&files)?));

    // A reload restores the last song with no decode.
    p.reload()?;
    let from = p.mark();
    let restored = p.wait_for(from, 120, |l| l.contains("Restored \"") || is_page_error(l))?;
    p.check(restored.is_some(), "the reload restores the last song");
    p.idle(1.0)?;
    let checks = stored_open_checks(&p.log[from..]);
    check_all(&mut p, checks);

    // A session of the M4 layout restores through the decode worker, and
    // is then complete.
    in_page(&mut p, PLANT_M4, json!(["1792640009999"]), 60)?;
    p.reload()?;
    let from = p.mark();
    let restored = p.wait_for(from, 120, |l| {
        l.contains("Restored \"m4\"") || is_page_error(l)
    })?;
    p.check(
        restored.is_some(),
        "the reload restores the session of the M4 layout",
    );
    p.idle(1.0)?;
    let checks = decoded_again_checks(&p.log[from..]);
    check_all(&mut p, checks);
    let got = in_page(&mut p, READ_SESSION, json!([false]), 120)?;
    check_all(&mut p, session_checks(&got, &want));

    let all: Vec<String> = in_page(&mut p, ALL_FILES, json!([]), 60)?
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    let (ok, what) = no_parts_check(&all);
    p.check(ok, &what);
    let (ok, what) = never_on_the_page_check(&p.log);
    p.check(ok, &what);
    let errors = p.errors();
    p.check(
        errors.is_empty(),
        &format!("no page errors ({})", errors.len()),
    );
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wav(seconds: f32, seed: usize, rate: u32) -> Vec<u8> {
        let frames = (seconds * rate as f32) as usize;
        let tone = |k: usize| -> Vec<f32> {
            (0..frames)
                .map(|i| 0.6 * (0.01 * (k + 1) as f32 * i as f32).sin())
                .collect()
        };
        engine::wav::encode_pcm16(&tone(seed), &tone(seed + 1), rate)
    }

    /// Base64 of `bytes`, as the page sends it.
    fn base64(bytes: &[u8]) -> String {
        const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn named(files: &[(&str, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
        files
            .iter()
            .map(|(n, b)| (n.to_string(), b.clone()))
            .collect()
    }

    #[test]
    fn one_source_is_expected_in_the_mix_file_and_several_in_their_own() {
        let one = expected(&named(&[("song.wav", wav(0.1, 0, 44_100))])).unwrap();
        assert_eq!(one.frames, 4_410);
        assert_eq!(one.hashes.keys().collect::<Vec<_>>(), ["mix.f32"]);
        assert_eq!(
            one.overviews.keys().collect::<Vec<_>>(),
            ["overviews/0.f32", "overviews/mix.f32"]
        );
        let several = expected(&named(&[
            ("vocals.wav", wav(0.1, 0, 44_100)),
            ("drums.wav", wav(0.2, 2, 44_100)),
            ("mix.wav", wav(0.3, 4, 44_100)),
        ]))
        .unwrap();
        assert_eq!(
            several.frames, 8_820,
            "the longest source; mix.wav is no source"
        );
        assert_eq!(
            several.hashes.keys().collect::<Vec<_>>(),
            ["decoded/0.f32", "decoded/1.f32", "mix.f32"]
        );
        assert_eq!(several.overviews.len(), 3);
        // A loud pair clips in the mix, so the mix is not the sum.
        assert_ne!(several.hashes["mix.f32"], several.hashes["decoded/1.f32"]);
    }

    /// What the page sends for `want`, as it would for a good session.
    fn page_reply(want: &Expected) -> Value {
        let mut files: Vec<Value> = want
            .hashes
            .keys()
            .chain(want.overviews.keys())
            .map(|p| json!([p, 1]))
            .collect();
        files.push(json!(["src/0.wav", 1]));
        files.push(json!(["meta", 1]));
        let overviews: BTreeMap<&String, String> =
            want.overviews.iter().map(|(p, b)| (p, base64(b))).collect();
        json!({
            "files": files,
            "hashes": want.hashes,
            "overviews": overviews,
            "meta": format!("name\tsong\nrate\t44100\nfile\tsong.wav\t0.wav\nmix\t{0}\tf32le-1\ndecoded\t{0}\tf32le-1\t0:2\n", want.frames),
        })
    }

    fn failing(checks: &Checks) -> Vec<String> {
        checks
            .iter()
            .filter(|(ok, _)| !ok)
            .map(|(_, w)| w.clone())
            .collect()
    }

    #[test]
    fn the_session_checks_pass_a_good_session_and_fail_wrong_ones() {
        let want = expected(&named(&[("song.wav", wav(0.1, 0, 44_100))])).unwrap();
        let good = page_reply(&want);
        let checks = session_checks(&good, &want);
        assert!(checks.len() >= 5, "{checks:?}");
        assert_eq!(failing(&checks), Vec::<String>::new());
        let mut bad_hash = good.clone();
        bad_hash["hashes"]["mix.f32"] = json!("00");
        assert_eq!(failing(&session_checks(&bad_hash, &want)).len(), 1);
        let mut bad_overview = good.clone();
        bad_overview["overviews"]["overviews/mix.f32"] = json!("AAAA");
        assert_eq!(failing(&session_checks(&bad_overview, &want)).len(), 1);
        let mut extra = good.clone();
        extra["files"]
            .as_array_mut()
            .unwrap()
            .push(json!(["decoded/0.f32", 8]));
        assert_eq!(
            failing(&session_checks(&extra, &want)).len(),
            1,
            "no decoded folder"
        );
        for line in ["mix\t", "decoded\t"] {
            let mut no_line = good.clone();
            let meta: String = good["meta"]
                .as_str()
                .unwrap()
                .lines()
                .filter(|l| !l.starts_with(line))
                .map(|l| format!("{l}\n"))
                .collect();
            no_line["meta"] = json!(meta);
            assert_eq!(
                failing(&session_checks(&no_line, &want)).len(),
                1,
                "{line:?}"
            );
        }
    }

    /// An overview's bytes are its columns of `engine::overview`, each its
    /// minimum then its maximum as 32-bit little-endian floats.
    #[test]
    fn the_expected_overviews_are_the_columns_of_engine() {
        assert_eq!(
            column_bytes(&[(-0.5, 0.25), (0.0, 1.0)]),
            [-0.5f32, 0.25, 0.0, 1.0]
                .iter()
                .flat_map(|x| x.to_le_bytes())
                .collect::<Vec<u8>>()
        );
        let bytes = wav(0.2, 0, 44_100);
        let want = expected(&named(&[("song.wav", bytes.clone())])).unwrap();
        let audio = engine::decode_bytes(bytes, Some("wav")).unwrap();
        let (cols, _) = lane_columns(&audio.channels[0], &audio.channels[1]);
        assert_eq!(cols.len(), engine::overview::column_count(want.frames));
        assert_eq!(want.overviews["overviews/0.f32"], column_bytes(&cols));
    }

    #[test]
    fn close_samples_pass_and_far_or_short_ones_fail() {
        let want = [0.5, -0.25, 0.125];
        assert!(close_check(&[0.5, -0.25, 0.125], &want, 1e-6).0);
        assert!(close_check(&[0.500_000_5, -0.25, 0.125], &want, 1e-6).0);
        assert!(!close_check(&[0.5, -0.25, 0.126], &want, 1e-6).0);
        let (ok, what) = close_check(&[0.5, -0.25], &want, 1e-6);
        assert!(!ok && what.contains("2 frames"), "{what}");
    }

    #[test]
    fn a_part_file_fails_the_check() {
        let clean = ["sessions/1/mix.f32", "sessions/1/meta", "model.part.not"].map(String::from);
        assert!(no_parts_check(&clean).0);
        for part in [
            "sessions/1/mix.f32.decode.part",
            "sessions/1/stems/bass.wav.part",
        ] {
            let mut with = clean.to_vec();
            with.push(part.into());
            let (ok, what) = no_parts_check(&with);
            assert!(!ok && what.contains(part), "{what}");
        }
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_log_checks_tell_a_stored_open_from_a_decode() {
        let stored = lines(&[
            "[1.0s] console.info: [app] decoder: a complete session opens as stored lanes",
            "[1.1s] console.info: [app] Restored \"song\" from this browser.",
        ]);
        let decoded = lines(&[
            "[1.0s] console.info: decoder: memory: decoder wasm 6 MB at the decode's start",
            "[1.4s] console.info: decoder: memory: decoder wasm 6 MB at the decode's end",
            "[1.5s] console.info: [app] Restored \"song\" from this browser.",
        ]);
        assert_eq!(failing(&stored_open_checks(&stored)), Vec::<String>::new());
        assert!(
            !failing(&stored_open_checks(&decoded)).is_empty(),
            "a decode ran"
        );
        assert_eq!(
            failing(&decoded_again_checks(&decoded)),
            Vec::<String>::new()
        );
        assert!(
            !failing(&decoded_again_checks(&stored)).is_empty(),
            "no decode"
        );
        assert!(never_on_the_page_check(&decoded).0);
        let mut on_page = decoded.clone();
        on_page.push("[2.0s] console.info: [app] decoder: decoding on the page".into());
        let (ok, what) = never_on_the_page_check(&on_page);
        assert!(!ok && what.contains("decoding on the page"), "{what}");
        assert!(!failing(&decoded_again_checks(&on_page)).is_empty());
        assert!(
            !failing(&stored_open_checks(
                &[stored.clone(), on_page.clone()].concat()
            ))
            .is_empty()
        );
    }
}
