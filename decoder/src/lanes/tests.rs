//! The requests over lane files, on a session in memory
//! (`cargo test -p decode-worker`).

#![expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]

use super::*;
use crate::pass::tests::{Memory, SESSION, overview_bytes, ready, tone};
use crate::pass::{BLOCK_BYTES, PART};
use engine::decode::decode_bytes;
use engine::hpss::{BlockHpss, HpssConfig};
use engine::overview::lane_columns;
use engine::song::padded_to;
use engine::wav::{HEADER_BYTES, encode_pcm16};
use stem_protocol::decode::{LaneEncoding, LaneRef, OverviewLane};

/// A stem WAV of `frames` frames, different for each `seed`, as the stem
/// worker writes one: 16-bit stereo at 44.1 kHz.
fn stem_wav(frames: usize, seed: usize) -> Vec<u8> {
    encode_pcm16(
        &tone(frames, 0.6, seed),
        &tone(frames, 0.5, seed + 7),
        44_100,
    )
}

/// The two sides of a stem WAV, as `read_stem` gave them: decoded, and
/// padded with silence to `frames`.
fn sides_of(wav: &[u8], frames: usize) -> (Vec<f32>, Vec<f32>) {
    let audio = decode_bytes(wav.to_vec(), Some("wav")).unwrap();
    let mut channels = audio.channels.into_iter();
    let mut side = || padded_to(channels.next().unwrap(), frames);
    (side(), side())
}

fn wav_lane(path: &str, wav: &[u8]) -> LaneRef {
    LaneRef {
        path: path.into(),
        encoding: LaneEncoding::Pcm16,
        data_offset: HEADER_BYTES as u64,
        data_frames: ((wav.len() - HEADER_BYTES) / 4) as u64,
    }
}

/// Six stems of the song's frames, and two one frame short, in the
/// session, with an `overviews` request for them and their mix.
fn six_stems(frames: usize) -> (Memory, Vec<Vec<u8>>, OverviewsRequest) {
    let memory = Memory::default();
    let wavs: Vec<Vec<u8>> = (0..6)
        .map(|k| stem_wav(if k % 3 == 0 { frames - 1 } else { frames }, k))
        .collect();
    let lanes = wavs
        .iter()
        .enumerate()
        .map(|(k, wav)| {
            let path = format!("stems/{k}.wav");
            memory.plant(&format!("{SESSION}/{path}"), wav.clone());
            OverviewLane {
                lane: wav_lane(&path, wav),
                overview: format!("overviews/stems/{k}.f32.3{PART}"),
            }
        })
        .collect();
    let req = OverviewsRequest {
        session: SESSION.into(),
        frames: frames as u64,
        lanes,
        mix: Some(format!("overviews/stems/mix.f32.3{PART}")),
    };
    (memory, wavs, req)
}

/// The clamped sum at volume 1 of `sides`, as `mix_of_sources` makes it
/// with `dsp::mix`.
fn mix_of(sides: &[(Vec<f32>, Vec<f32>)]) -> (Vec<f32>, Vec<f32>) {
    let ones = vec![1.0; sides.len()];
    let lefts: Vec<&[f32]> = sides.iter().map(|(l, _)| l.as_slice()).collect();
    let rights: Vec<&[f32]> = sides.iter().map(|(_, r)| r.as_slice()).collect();
    (dsp::mix(&lefts, &ones), dsp::mix(&rights, &ones))
}

/// D7: the overview of each stem, and of their mix, equal `lane_columns`
/// of the samples that the page reads, bit for bit.
#[test]
fn stem_overviews_equal_lane_columns() {
    // More than a block of 131,072 frames, and not a whole number of them.
    let frames = 300_007;
    let (mut memory, wavs, req) = six_stems(frames);
    let reply = ready(overviews(&mut memory, &req, &mut Buffers::default()));
    assert_eq!(reply, DecodeReply::OverviewsMade);
    let sides: Vec<_> = wavs.iter().map(|wav| sides_of(wav, frames)).collect();
    for (k, (left, right)) in sides.iter().enumerate() {
        let (cols, _) = lane_columns(left, right);
        let got = memory.file(&format!("overviews/stems/{k}.f32.3{PART}"));
        assert!(got == Some(overview_bytes(&cols)), "stem {k}");
    }
    let (ml, mr) = mix_of(&sides);
    let (cols, _) = lane_columns(&ml, &mr);
    assert!(
        memory.file(&format!("overviews/stems/mix.f32.3{PART}")) == Some(overview_bytes(&cols))
    );
    // Each read is at most 1 MiB, and so is each write.
    assert!(memory.largest_read.get() <= BLOCK_BYTES);
    assert!(
        memory
            .write_sizes
            .borrow()
            .iter()
            .all(|size| *size <= BLOCK_BYTES)
    );
}

/// D8: the WAV is the block HPSS of the lanes' mix, rounded to 16 bits,
/// and its overview is `lane_columns` of the WAV's samples.
#[test]
fn a_classic_request_writes_the_block_hpss_as_a_wav() {
    let frames = 150_001;
    let memory = Memory::default();
    // A stem WAV one frame short, and an f32 session file.
    let wav = stem_wav(frames - 1, 2);
    memory.plant(&format!("{SESSION}/stems/0.wav"), wav.clone());
    let (fl, fr) = (tone(frames, 0.7, 4), tone(frames, 0.7, 5));
    let f32_bytes: Vec<u8> = fl
        .iter()
        .zip(&fr)
        .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
        .flatten()
        .collect();
    memory.plant(&format!("{SESSION}/decoded/1.f32"), f32_bytes);
    let req = ClassicRequest {
        session: SESSION.into(),
        frames: frames as u64,
        lanes: vec![
            wav_lane("stems/0.wav", &wav),
            LaneRef {
                path: "decoded/1.f32".into(),
                encoding: LaneEncoding::F32,
                data_offset: 0,
                data_frames: frames as u64,
            },
        ],
        wav: format!("classic.wav{PART}"),
        overview: format!("overviews/classic.f32{PART}"),
    };
    let mut memory = memory;
    let reply = ready(classic(&mut memory, &req, &mut Buffers::default()));
    assert_eq!(reply, DecodeReply::ClassicMade);

    let (ml, mr) = mix_of(&[sides_of(&wav, frames), (fl, fr)]);
    let mut hpss = BlockHpss::new(frames, &HpssConfig::default());
    let (mut pl, mut pr) = (Vec::new(), Vec::new());
    let mut sink = |l: &[f32], r: &[f32]| -> Result<(), ()> {
        pl.extend_from_slice(l);
        pr.extend_from_slice(r);
        Ok(())
    };
    for (l, r) in ml.chunks(32_768).zip(mr.chunks(32_768)) {
        hpss.put(l, r, &mut sink).unwrap();
    }
    hpss.finish(&mut sink).unwrap();
    let want = encode_pcm16(&pl, &pr, 44_100);
    let got = memory.part("classic.wav");
    assert!(got == want, "the WAV of the block HPSS, rounded to 16 bits");
    let (left, right) = sides_of(&got, frames);
    let (cols, _) = lane_columns(&left, &right);
    assert!(memory.part("overviews/classic.f32") == overview_bytes(&cols));
    assert!(memory.largest_read.get() <= BLOCK_BYTES);
    assert!(
        memory
            .write_sizes
            .borrow()
            .iter()
            .all(|size| *size <= BLOCK_BYTES)
    );
}

fn is_unstored(reply: &DecodeReply, containing: &str) -> bool {
    matches!(reply, DecodeReply::Unstored { text } if text.contains(containing))
}

fn no_part_is_left(memory: &Memory, case: &str) {
    assert_eq!(memory.parts(), Vec::<String>::new(), "{case}");
}

fn classic_of(frames: usize) -> (Memory, ClassicRequest) {
    let memory = Memory::default();
    let wav = stem_wav(frames, 1);
    memory.plant(&format!("{SESSION}/stems/0.wav"), wav.clone());
    let req = ClassicRequest {
        session: SESSION.into(),
        frames: frames as u64,
        lanes: vec![wav_lane("stems/0.wav", &wav)],
        wav: format!("classic.wav{PART}"),
        overview: format!("overviews/classic.f32{PART}"),
    };
    (memory, req)
}

/// Each write that fails, a missing lane file, and a lane file shorter
/// than its frames end the request with `decode-unstored`, and leave no
/// part file.
#[test]
fn a_failed_read_or_write_ends_with_unstored_and_leaves_no_part_file() {
    let frames = 140_000;
    let writes_of = |memory: &Memory| memory.write_sizes.borrow().len();
    let (mut all, _, req) = six_stems(frames);
    assert_eq!(
        ready(overviews(&mut all, &req, &mut Buffers::default())),
        DecodeReply::OverviewsMade
    );
    let writes = writes_of(&all);
    assert_eq!(writes, 7, "six stems and the mix");
    for fail in 1..=writes {
        let (mut memory, _, req) = six_stems(frames);
        memory.fail_write = Some(fail);
        let reply = ready(overviews(&mut memory, &req, &mut Buffers::default()));
        assert!(
            is_unstored(&reply, "QuotaExceededError"),
            "overviews, write {fail}: {reply:?}"
        );
        no_part_is_left(&memory, &format!("overviews, write {fail}"));
    }
    let (mut all, req) = classic_of(frames);
    assert_eq!(
        ready(classic(&mut all, &req, &mut Buffers::default())),
        DecodeReply::ClassicMade
    );
    let writes = writes_of(&all);
    assert!(
        writes >= 3,
        "the header, the frames and the overview: {writes}"
    );
    for fail in 1..=writes {
        let (mut memory, req) = classic_of(frames);
        memory.fail_write = Some(fail);
        let reply = ready(classic(&mut memory, &req, &mut Buffers::default()));
        assert!(
            is_unstored(&reply, "QuotaExceededError"),
            "classic, write {fail}: {reply:?}"
        );
        no_part_is_left(&memory, &format!("classic, write {fail}"));
    }
    // A missing lane file, and one shorter than its frames.
    let (mut memory, _, req) = six_stems(frames);
    memory
        .files
        .borrow_mut()
        .remove(&format!("{SESSION}/stems/2.wav"));
    let reply = ready(overviews(&mut memory, &req, &mut Buffers::default()));
    assert!(is_unstored(&reply, "NotFoundError"), "{reply:?}");
    no_part_is_left(&memory, "a missing stem");
    let (mut memory, req) = classic_of(frames);
    memory
        .files
        .borrow_mut()
        .get_mut(&format!("{SESSION}/stems/0.wav"))
        .unwrap()
        .truncate(1_000);
    let reply = ready(classic(&mut memory, &req, &mut Buffers::default()));
    assert!(is_unstored(&reply, "stems/0.wav"), "{reply:?}");
    no_part_is_left(&memory, "a short stem");
}
