//! The requests over lane files (`docs/plans/active/stream-from-storage-m7.md`,
//! D7 and D8): the overviews of stems, and classic drums.
//!
//! Both read each lane in blocks of at most [`BLOCK_FRAMES`] frames, with
//! `FileReaderSync` in the browser (`Session::stored`), never through a
//! sync access handle, so a running separation's reads see no lock.  A lane
//! is silence after the frames its file holds.  The mix of the lanes is
//! their sum at volume 1, clamped, as `dsp::mix` makes it.  On a failure
//! the request removes every part file that it names, and ends with
//! `decode-unstored`.  The memory of a request does not grow with the song.

use crate::pass::{BLOCK_BYTES, BLOCK_FRAMES, Buffers, PartFile, Session, StoredFile, clamped};
use engine::hpss::{BlockHpss, HpssConfig};
use engine::overview::OverviewBuilder;
use engine::readahead::{LaneFormat, decode_frames};
use engine::wav::{FRAME_BYTES, HEADER_BYTES, header, write_pcm16};
use stem_protocol::decode::{ClassicRequest, DecodeReply, LaneEncoding, LaneRef, OverviewsRequest};

/// The rate of a classic-drums WAV (decision 0016).
const CLASSIC_RATE: u32 = 44_100;

/// Writes the overview of each lane, and of their mix if asked.
pub async fn overviews<S: Session>(
    session: &mut S,
    req: &OverviewsRequest,
    bufs: &mut Buffers,
) -> DecodeReply {
    let parts: Vec<String> = req
        .lanes
        .iter()
        .map(|lane| lane.overview.clone())
        .chain(req.mix.clone())
        .collect();
    match make_overviews(session, req, bufs).await {
        Ok(()) => DecodeReply::OverviewsMade,
        Err(text) => fail(session, &req.session, &parts, text).await,
    }
}

/// Writes classic drums of the lanes' mix as a 16-bit WAV, and its
/// overview.
pub async fn classic<S: Session>(
    session: &mut S,
    req: &ClassicRequest,
    bufs: &mut Buffers,
) -> DecodeReply {
    match make_classic(session, req, bufs).await {
        Ok(()) => DecodeReply::ClassicMade,
        Err(text) => {
            let parts = [req.wav.clone(), req.overview.clone()];
            fail(session, &req.session, &parts, text).await
        }
    }
}

/// Removes the part files `parts` of the session folder `folder`, and
/// gives `decode-unstored` with `text`.
async fn fail<S: Session>(
    session: &mut S,
    folder: &str,
    parts: &[String],
    text: String,
) -> DecodeReply {
    for part in parts {
        session.remove(&format!("{folder}/{part}")).await;
    }
    DecodeReply::Unstored { text }
}

fn format_of(encoding: LaneEncoding) -> LaneFormat {
    match encoding {
        LaneEncoding::F32 => LaneFormat::F32,
        LaneEncoding::Pcm16 => LaneFormat::Pcm16,
    }
}

/// Opens each lane file of the session folder `folder`, and checks that it
/// holds the frames that its lane names.
async fn open_lanes<S: Session>(
    session: &mut S,
    folder: &str,
    lanes: &[LaneRef],
) -> Result<Vec<(LaneRef, S::Stored)>, String> {
    let mut open = Vec::with_capacity(lanes.len());
    for lane in lanes {
        let file = session.stored(&format!("{folder}/{}", lane.path)).await?;
        let bytes = format_of(lane.encoding).bytes_per_frame() as u64;
        let end = lane.data_offset + lane.data_frames * bytes;
        if file.size() < end {
            return Err(format!(
                "{} holds {} bytes, less than its {} frames",
                lane.path,
                file.size(),
                lane.data_frames
            ));
        }
        open.push((lane.clone(), file));
    }
    Ok(open)
}

/// Reads frames `start..start + left.len()` of `lane` into `left` and
/// `right`, with silence after the frames that its file holds.
fn read_lane<F: StoredFile>(
    lane: &LaneRef,
    file: &mut F,
    start: u64,
    read: &mut [u8],
    left: &mut [f32],
    right: &mut [f32],
) -> Result<(), String> {
    let format = format_of(lane.encoding);
    let per_frame = format.bytes_per_frame();
    let have = lane
        .data_frames
        .saturating_sub(start)
        .min(left.len() as u64) as usize;
    if have > 0 {
        let bytes = &mut read[..have * per_frame];
        file.read_at(lane.data_offset + start * per_frame as u64, bytes)
            .map_err(|text| format!("{} could not be read: {text}", lane.path))?;
        decode_frames(format, bytes, &mut left[..have], &mut right[..have]);
    }
    left[have..].fill(0.0);
    right[have..].fill(0.0);
    Ok(())
}

/// Reads `n` frames from `start` of each lane, gives each lane's frames to
/// `each`, and leaves their mix in `bufs.mix_left` and `bufs.mix_right`:
/// the sum at volume 1 from 0, in the lanes' order, then clamped, as
/// `dsp::mix` makes it.
fn mix_block<F: StoredFile>(
    lanes: &mut [(LaneRef, F)],
    start: u64,
    n: usize,
    bufs: &mut Buffers,
    mut each: impl FnMut(usize, &[f32], &[f32]),
) -> Result<(), String> {
    bufs.mix_left[..n].fill(0.0);
    bufs.mix_right[..n].fill(0.0);
    for (k, (lane, file)) in lanes.iter_mut().enumerate() {
        let (left, right) = (&mut bufs.left[..n], &mut bufs.right[..n]);
        read_lane(lane, file, start, &mut bufs.read, left, right)?;
        each(k, left, right);
        for i in 0..n {
            bufs.mix_left[i] += left[i];
            bufs.mix_right[i] += right[i];
        }
    }
    for i in 0..n {
        bufs.mix_left[i] = clamped(bufs.mix_left[i]);
        bufs.mix_right[i] = clamped(bufs.mix_right[i]);
    }
    Ok(())
}

async fn make_overviews<S: Session>(
    session: &mut S,
    req: &OverviewsRequest,
    bufs: &mut Buffers,
) -> Result<(), String> {
    let refs: Vec<LaneRef> = req.lanes.iter().map(|l| l.lane.clone()).collect();
    let mut lanes = open_lanes(session, &req.session, &refs).await?;
    let frames = req.frames;
    let mut builders: Vec<OverviewBuilder> = (0..lanes.len())
        .map(|_| OverviewBuilder::new(frames as usize))
        .collect();
    let mut mix = OverviewBuilder::new(frames as usize);
    let mut start = 0;
    while start < frames {
        let n = (frames - start).min(BLOCK_FRAMES as u64) as usize;
        mix_block(&mut lanes, start, n, bufs, |k, left, right| {
            builders[k].put(left, right);
        })?;
        mix.put(&bufs.mix_left[..n], &bufs.mix_right[..n]);
        start += n as u64;
    }
    for (lane, builder) in req.lanes.iter().zip(builders) {
        let path = format!("{}/{}", req.session, lane.overview);
        write_overview(session, &path, builder, &mut bufs.staging).await?;
    }
    if let Some(part) = &req.mix {
        let path = format!("{}/{part}", req.session);
        write_overview(session, &path, mix, &mut bufs.staging).await?;
    }
    Ok(())
}

/// Writes the columns of `overview` to a new part file at `path`: for each
/// column its minimum, then its maximum, as 32-bit little-endian floats
/// (D1 of M5), in one write of at most 32 KiB.
async fn write_overview<S: Session>(
    session: &mut S,
    path: &str,
    overview: OverviewBuilder,
    staging: &mut Vec<u8>,
) -> Result<(), String> {
    let (columns, _) = overview.finish();
    staging.clear();
    for (lo, hi) in &columns {
        staging.extend_from_slice(&lo.to_le_bytes());
        staging.extend_from_slice(&hi.to_le_bytes());
    }
    let mut part = session.create(path).await?;
    let written = part.write_at(0, staging);
    let closed = session.close(part).await;
    written.map_err(|text| format!("an overview could not be written: {text}"))?;
    closed.map_err(|text| format!("an overview could not be closed: {text}"))
}

async fn make_classic<S: Session>(
    session: &mut S,
    req: &ClassicRequest,
    bufs: &mut Buffers,
) -> Result<(), String> {
    let mut lanes = open_lanes(session, &req.session, &req.lanes).await?;
    let wav_path = format!("{}/{}", req.session, req.wav);
    let mut wav = session.create(&wav_path).await?;
    let written = write_classic(&mut lanes, &mut wav, req.frames, bufs);
    let closed = session.close(wav).await;
    written?;
    closed.map_err(|text| format!("the classic drums could not be closed: {text}"))?;
    // The overview of the WAV, read back in blocks, as the page reads it.
    let lane = LaneRef {
        path: req.wav.clone(),
        encoding: LaneEncoding::Pcm16,
        data_offset: HEADER_BYTES as u64,
        data_frames: req.frames,
    };
    let mut written = open_lanes(session, &req.session, std::slice::from_ref(&lane)).await?;
    let (_, file) = &mut written[0];
    let mut builder = OverviewBuilder::new(req.frames as usize);
    let mut start = 0;
    while start < req.frames {
        let n = (req.frames - start).min(BLOCK_FRAMES as u64) as usize;
        let (left, right) = (&mut bufs.left[..n], &mut bufs.right[..n]);
        read_lane(&lane, file, start, &mut bufs.read, left, right)?;
        builder.put(left, right);
        start += n as u64;
    }
    let path = format!("{}/{}", req.session, req.overview);
    write_overview(session, &path, builder, &mut bufs.staging).await
}

/// Writes the WAV of classic drums: its header, then the percussive part
/// of the lanes' mix, made by a block HPSS, as 16-bit frames, in writes of
/// at most a block.
fn write_classic<F: StoredFile, P: PartFile>(
    lanes: &mut [(LaneRef, F)],
    wav: &mut P,
    frames: u64,
    bufs: &mut Buffers,
) -> Result<(), String> {
    let write_failed = |text: String| format!("the classic drums could not be written: {text}");
    wav.write_at(0, &header(frames as usize, CLASSIC_RATE))
        .map_err(write_failed)?;
    let mut hpss = BlockHpss::new(frames as usize, &HpssConfig::default());
    let mut at = HEADER_BYTES as u64;
    bufs.staging.clear();
    let mut start = 0;
    while start < frames {
        let n = (frames - start).min(BLOCK_FRAMES as u64) as usize;
        mix_block(lanes, start, n, bufs, |_, _, _| {})?;
        let Buffers {
            staging,
            mix_left,
            mix_right,
            ..
        } = &mut *bufs;
        let mut sink = |left: &[f32], right: &[f32]| put_pcm16(wav, staging, &mut at, left, right);
        hpss.put(&mix_left[..n], &mix_right[..n], &mut sink)
            .map_err(write_failed)?;
        start += n as u64;
    }
    let staging = &mut bufs.staging;
    let mut sink = |left: &[f32], right: &[f32]| put_pcm16(wav, staging, &mut at, left, right);
    hpss.finish(&mut sink).map_err(write_failed)?;
    flush(wav, staging, &mut at).map_err(write_failed)
}

/// Adds frames to `staging` as 16-bit frames, and writes `staging` at
/// `at` before it would pass a block.
fn put_pcm16<P: PartFile>(
    wav: &mut P,
    staging: &mut Vec<u8>,
    at: &mut u64,
    left: &[f32],
    right: &[f32],
) -> Result<(), String> {
    let bytes = left.len() * FRAME_BYTES;
    if staging.len() + bytes > BLOCK_BYTES {
        flush(wav, staging, at)?;
    }
    let from = staging.len();
    staging.resize(from + bytes, 0);
    write_pcm16(left, right, &mut staging[from..]);
    Ok(())
}

fn flush<P: PartFile>(wav: &mut P, staging: &mut Vec<u8>, at: &mut u64) -> Result<(), String> {
    if staging.is_empty() {
        return Ok(());
    }
    wav.write_at(*at, staging)?;
    *at += staging.len() as u64;
    staging.clear();
    Ok(())
}

#[cfg(test)]
mod tests;
