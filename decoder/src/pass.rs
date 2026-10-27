//! The decode of one request into the session files
//! (`docs/plans/active/stream-from-storage-m5.md`, D1, D5, D7, D7a, D8).
//!
//! The pass runs over a [`Session`], so the native tests run it on files
//! in memory and the browser runs it on OPFS.  It writes each file of D1
//! as `<name>.decode.part`; the page moves each one onto its name.
//!
//! * The first pass decodes each source with `engine::song::decode_into`
//!   into its file, in blocks.  A song of one source decodes the source in
//!   the mix into `mix.f32`; every other source goes to
//!   `decoded/<i>.f32`.
//! * The second pass reads the files back in blocks.  It pads each file
//!   with silence to the song's frames, writes `mix.f32` for a song of
//!   several sources, and makes every overview.
//! * With `mix_frames`, a usable `mix.f32` stays (D8): the pass writes no
//!   mix, and for a song of one source it reads no source in the mix.
//!
//! The pass holds the [`Buffers`] that the worker makes once, and no more
//! than a block of any file, so its memory does not grow with the song.

use crate::source::ReadFailure;
use engine::decode::{Decoder, MediaSource};
use engine::overview::{OverviewBuilder, column_count};
use engine::song::{FrameSink, decode_into};
use stem_protocol::decode::{DecodeReply, DecodeRequest};

/// Frames in each block that the pass writes or reads: 1 MiB of
/// interleaved stereo 32-bit floats.
pub const BLOCK_FRAMES: usize = 131_072;

/// Bytes in one frame of a session file: two 32-bit floats.
const FRAME_BYTES: u64 = 8;

/// Bytes in one block.
pub(crate) const BLOCK_BYTES: usize = BLOCK_FRAMES * FRAME_BYTES as usize;

/// Bytes in one column of an overview file: its minimum and its maximum.
const COLUMN_BYTES: usize = 8;

/// The suffix of each file that the decode worker writes.
pub const PART: &str = ".decode.part";

/// A file that the pass writes, and reads back in the second pass.
pub trait PartFile {
    /// Writes `bytes` at byte `at`.
    fn write_at(&mut self, at: u64, bytes: &[u8]) -> Result<(), String>;
    /// Fills `buf` with the bytes from byte `at`.
    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> Result<(), String>;
}

/// A stored file that the pass reads only: a kept `mix.f32`.
pub trait StoredFile {
    /// The bytes of the file.
    fn size(&self) -> u64;
    /// Fills `buf` with the bytes from byte `at`.
    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> Result<(), String>;
}

/// The files of one session.  Each path is from the root of the private
/// file system.  An error is a text for `decode-unstored`.
#[expect(
    async_fn_in_trait,
    reason = "only this crate implements Session, and the worker runs on one thread"
)]
pub trait Session {
    type Part: PartFile;
    type Stored: StoredFile;
    /// A stored source, for the decoder.  A read that fails is recorded in
    /// `failure`.
    async fn source(
        &mut self,
        path: &str,
        failure: ReadFailure,
    ) -> Result<Box<dyn MediaSource>, String>;
    /// A stored file to read.
    async fn stored(&mut self, path: &str) -> Result<Self::Stored, String>;
    /// An empty file to write, in a folder that is made if needed.
    async fn create(&mut self, path: &str) -> Result<Self::Part, String>;
    /// Closes a file.  A file is closed before the reply, because the page
    /// cannot move a file while a handle is open on it.
    async fn close(&mut self, part: Self::Part) -> Result<(), String>;
    /// Removes the file at `path`, if it is there.
    async fn remove(&mut self, path: &str);
}

/// The working set of the pass, made once when the worker starts, so a
/// decode makes no buffer of its own (D12).
pub struct Buffers {
    /// The interleaved bytes of the next write, at most a block.
    pub(crate) staging: Vec<u8>,
    /// The bytes of one block that the second pass reads.
    pub(crate) read: Vec<u8>,
    pub(crate) left: Vec<f32>,
    pub(crate) right: Vec<f32>,
    pub(crate) mix_left: Vec<f32>,
    pub(crate) mix_right: Vec<f32>,
}

impl Default for Buffers {
    fn default() -> Self {
        let samples = || vec![0.0; BLOCK_FRAMES];
        Self {
            staging: Vec::with_capacity(BLOCK_BYTES),
            read: vec![0; BLOCK_BYTES],
            left: samples(),
            right: samples(),
            mix_left: samples(),
            mix_right: samples(),
        }
    }
}

/// Where the samples of one source are.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Lane {
    /// The part file at this index of the pass's open files.
    Part(usize),
    /// The kept `mix.f32` (D8).
    KeptMix,
}

/// The paths of a request's files, from the root of the private file
/// system, without [`PART`].
fn mix_path(session: &str) -> String {
    format!("{session}/mix.f32")
}

fn decoded_path(session: &str, index: u32) -> String {
    format!("{session}/decoded/{index}.f32")
}

fn overview_path(session: &str, name: &str) -> String {
    format!("{session}/overviews/{name}.f32")
}

/// Why a decode ended without the session files.
enum Failed {
    Undecodable(String),
    Unstored(String),
}

/// The part files that the pass has open, with their paths.
struct Open<P> {
    files: Vec<(String, P)>,
}

impl<P: PartFile> Open<P> {
    async fn create<S: Session<Part = P>>(
        &mut self,
        session: &mut S,
        path: String,
    ) -> Result<usize, Failed> {
        let part = session.create(&path).await.map_err(Failed::Unstored)?;
        self.files.push((path, part));
        Ok(self.files.len() - 1)
    }
}

/// Writes interleaved stereo frames to a part file, a block at a time.
struct PartSink<'a, P> {
    part: &'a mut P,
    staging: &'a mut Vec<u8>,
    /// The byte where the staged bytes go.
    at: u64,
    /// The first write that failed: a storage failure, not a decode one.
    failed: Option<String>,
}

impl<P: PartFile> PartSink<'_, P> {
    fn flush(&mut self) -> Result<(), String> {
        if self.staging.is_empty() {
            return Ok(());
        }
        self.part.write_at(self.at, self.staging)?;
        self.at += self.staging.len() as u64;
        self.staging.clear();
        Ok(())
    }
}

impl<P: PartFile> FrameSink for PartSink<'_, P> {
    fn put(&mut self, left: &[f32], right: &[f32]) -> anyhow::Result<()> {
        for (l, r) in left.iter().zip(right) {
            self.staging.extend_from_slice(&l.to_le_bytes());
            self.staging.extend_from_slice(&r.to_le_bytes());
            if self.staging.len() >= BLOCK_BYTES
                && let Err(text) = self.flush()
            {
                self.failed = Some(text.clone());
                return Err(anyhow::anyhow!(text));
            }
        }
        Ok(())
    }
}

/// Decodes `req` into its session's part files, and gives the reply that
/// ends it.  On `undecodable` or `decode-unstored` it removes its part
/// files.  The pass that it leaves is `D7` on the page.
pub async fn run<S: Session>(
    session: &mut S,
    req: &DecodeRequest,
    bufs: &mut Buffers,
) -> DecodeReply {
    let mut open = Open { files: Vec::new() };
    let outcome = decode(session, req, bufs, &mut open).await;
    let mut close_failed = None;
    let paths: Vec<String> = open.files.iter().map(|(path, _)| path.clone()).collect();
    for (_, part) in open.files {
        if let Err(text) = session.close(part).await {
            close_failed.get_or_insert(text);
        }
    }
    let outcome = match (outcome, close_failed) {
        (Ok(DecodeReply::Decoded { .. }), Some(text)) => Err(Failed::Unstored(format!(
            "a file could not be closed: {text}"
        ))),
        (outcome, _) => outcome,
    };
    match outcome {
        Ok(reply) => reply,
        Err(failed) => {
            for path in planned_parts(req).into_iter().chain(paths) {
                session.remove(&path).await;
            }
            match failed {
                Failed::Undecodable(text) => DecodeReply::Undecodable { text },
                Failed::Unstored(text) => DecodeReply::Unstored { text },
            }
        }
    }
}

/// Every part file that `req` can make.
fn planned_parts(req: &DecodeRequest) -> Vec<String> {
    let s = &req.session;
    let mut paths = vec![mix_path(s), overview_path(s, "mix")];
    for source in &req.sources {
        paths.push(decoded_path(s, source.index));
        paths.push(overview_path(s, &source.index.to_string()));
    }
    paths.into_iter().map(|p| p + PART).collect()
}

async fn decode<S: Session>(
    session: &mut S,
    req: &DecodeRequest,
    bufs: &mut Buffers,
    open: &mut Open<S::Part>,
) -> Result<DecodeReply, Failed> {
    if req.sources.is_empty() {
        return Err(Failed::Undecodable("no stems to load".into()));
    }
    let s = &req.session;
    let in_mix = req.sources.iter().filter(|source| source.in_mix).count();
    let one_source = in_mix == 1;
    let kept = req.mix_frames;

    // The first pass: decode each source into its file.
    let mut lanes = Vec::with_capacity(req.sources.len());
    let mut frames = Vec::with_capacity(req.sources.len());
    let mut channels = Vec::with_capacity(req.sources.len());
    let mut kept_mix = None;
    for source in &req.sources {
        if one_source && source.in_mix && kept.is_some() {
            let path = mix_path(s);
            let file = session.stored(&path).await.map_err(Failed::Unstored)?;
            kept_mix = Some(file);
            lanes.push(Lane::KeptMix);
            frames.push(None);
            channels.push(0);
            continue;
        }
        let path = if one_source && source.in_mix {
            mix_path(s)
        } else {
            decoded_path(s, source.index)
        };
        let failure = ReadFailure::default();
        let unstored = |failure: &ReadFailure| {
            failure
                .text()
                .map(|text| Failed::Unstored(format!("{}: {text}", source.file)))
        };
        let media = session
            .source(&format!("{s}/{}", source.file), failure.clone())
            .await
            .map_err(Failed::Unstored)?;
        let decoder = match Decoder::new(media, Some(&source.ext)) {
            Ok(decoder) => decoder,
            Err(e) => {
                return Err(unstored(&failure).unwrap_or(Failed::Undecodable(format!("{e:#}"))));
            }
        };
        let at = open.create(session, path + PART).await?;
        let mut sink = PartSink {
            part: &mut open.files[at].1,
            staging: &mut bufs.staging,
            at: 0,
            failed: None,
        };
        sink.staging.clear();
        let decoded = decode_into(decoder, &mut sink);
        if let Some(failed) = unstored(&failure) {
            return Err(failed);
        }
        if let Some(text) = sink.failed.take() {
            return Err(Failed::Unstored(format!(
                "a session file could not be written: {text}"
            )));
        }
        let decoded = decoded.map_err(|e| Failed::Undecodable(format!("{e:#}")))?;
        sink.flush().map_err(|text| {
            Failed::Unstored(format!("a session file could not be written: {text}"))
        })?;
        lanes.push(Lane::Part(at));
        frames.push(Some(decoded.frames as u64));
        channels.push(decoded.channels as u8);
    }

    // The song's frames, and the checks of D8.
    let longest = frames.iter().flatten().copied().max().unwrap_or(0);
    let song_frames = match kept {
        None => longest,
        Some(kept) => {
            if one_source && longest > kept {
                return Err(Failed::Unstored(format!(
                    "a derived source has {longest} frames, more than the {kept} frames of the kept mix"
                )));
            }
            if !one_source && longest != kept {
                return Err(Failed::Unstored(format!(
                    "the sources have {longest} frames, not the {kept} frames of the kept mix"
                )));
            }
            kept
        }
    };
    if let Some(file) = &kept_mix {
        let want = FRAME_BYTES * song_frames;
        if file.size() != want {
            return Err(Failed::Unstored(format!(
                "the mix file holds {} bytes, not the {want} bytes of {song_frames} frames",
                file.size()
            )));
        }
    }

    // The second pass: pad each file, write a new mix, make the overviews.
    for (lane, decoded) in lanes.iter().zip(&frames) {
        if let (Lane::Part(at), Some(decoded)) = (lane, decoded) {
            pad(
                &mut open.files[*at].1,
                &mut bufs.staging,
                *decoded,
                song_frames,
            )?;
        }
    }
    let new_mix = kept.is_none();
    let write_mix = new_mix && !one_source;
    let mix_part = if write_mix {
        Some(open.create(session, mix_path(s) + PART).await?)
    } else {
        None
    };
    let columns = column_count(song_frames as usize);
    let mut overviews: Vec<OverviewBuilder> = (0..req.sources.len())
        .map(|_| OverviewBuilder::new(song_frames as usize))
        .collect();
    let mut mix_overview = OverviewBuilder::new(song_frames as usize);
    let mut kept_mix_is_mono = true;
    let mut start = 0;
    while start < song_frames {
        let n = (song_frames - start).min(BLOCK_FRAMES as u64) as usize;
        let bytes = n * FRAME_BYTES as usize;
        bufs.mix_left[..n].fill(0.0);
        bufs.mix_right[..n].fill(0.0);
        for (k, lane) in lanes.iter().enumerate() {
            let read = &mut bufs.read[..bytes];
            let at = start * FRAME_BYTES;
            match lane {
                Lane::Part(i) => open.files[*i].1.read_at(at, read),
                Lane::KeptMix => kept_mix
                    .as_mut()
                    .map_or(Ok(()), |file| file.read_at(at, read)),
            }
            .map_err(|text| {
                Failed::Unstored(format!("a session file could not be read: {text}"))
            })?;
            deinterleave(read, &mut bufs.left[..n], &mut bufs.right[..n]);
            let (left, right) = (&bufs.left[..n], &bufs.right[..n]);
            overviews[k].put(left, right);
            if *lane == Lane::KeptMix {
                kept_mix_is_mono &= left
                    .iter()
                    .zip(right)
                    .all(|(l, r)| l.to_bits() == r.to_bits());
            }
            if req.sources[k].in_mix {
                // As `dsp::mix` sums at volume 1: from 0, in the sources'
                // order, then clamped.
                for i in 0..n {
                    bufs.mix_left[i] += left[i];
                    bufs.mix_right[i] += right[i];
                }
            }
        }
        for i in 0..n {
            bufs.mix_left[i] = clamped(bufs.mix_left[i]);
            bufs.mix_right[i] = clamped(bufs.mix_right[i]);
        }
        let (mix_left, mix_right) = (&bufs.mix_left[..n], &bufs.mix_right[..n]);
        mix_overview.put(mix_left, mix_right);
        if let Some(at) = mix_part {
            bufs.staging.clear();
            for (l, r) in mix_left.iter().zip(mix_right) {
                bufs.staging.extend_from_slice(&l.to_le_bytes());
                bufs.staging.extend_from_slice(&r.to_le_bytes());
            }
            open.files[at]
                .1
                .write_at(start * FRAME_BYTES, &bufs.staging)
                .map_err(|text| {
                    Failed::Unstored(format!("the mix file could not be written: {text}"))
                })?;
        }
        start += n as u64;
    }
    for (source, overview) in req.sources.iter().zip(overviews) {
        let path = overview_path(s, &source.index.to_string()) + PART;
        write_overview(session, open, path, overview, columns, &mut bufs.staging).await?;
    }
    let path = overview_path(s, "mix") + PART;
    write_overview(
        session,
        open,
        path,
        mix_overview,
        columns,
        &mut bufs.staging,
    )
    .await?;

    for (lane, channel) in lanes.iter().zip(channels.iter_mut()) {
        if *lane == Lane::KeptMix {
            *channel = if kept_mix_is_mono { 1 } else { 2 };
        }
    }
    Ok(DecodeReply::Decoded {
        frames: song_frames,
        new_mix,
        channels,
    })
}

/// `dsp::mix`'s clamp to ±1, which keeps a NaN and the sign of 0, as
/// `f32::clamp` does.
pub(crate) fn clamped(x: f32) -> f32 {
    x.clamp(-1.0, 1.0)
}

fn deinterleave(bytes: &[u8], left: &mut [f32], right: &mut [f32]) {
    for ((frame, l), r) in bytes.chunks_exact(8).zip(left).zip(right) {
        *l = f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
        *r = f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
    }
}

/// Pads `part`, which holds `frames` frames, with silence to `to` frames,
/// a block at a time.
fn pad<P: PartFile>(
    part: &mut P,
    staging: &mut Vec<u8>,
    frames: u64,
    to: u64,
) -> Result<(), Failed> {
    let mut at = frames;
    while at < to {
        let n = (to - at).min(BLOCK_FRAMES as u64);
        staging.clear();
        staging.resize((n * FRAME_BYTES) as usize, 0);
        part.write_at(at * FRAME_BYTES, staging).map_err(|text| {
            Failed::Unstored(format!("a session file could not be padded: {text}"))
        })?;
        at += n;
    }
    Ok(())
}

/// Writes the columns of `overview` to a new part file at `path`: for each
/// column its minimum, then its maximum, as 32-bit little-endian floats.
async fn write_overview<S: Session>(
    session: &mut S,
    open: &mut Open<S::Part>,
    path: String,
    overview: OverviewBuilder,
    columns: usize,
    staging: &mut Vec<u8>,
) -> Result<(), Failed> {
    let (cols, _) = overview.finish();
    debug_assert_eq!(cols.len(), columns);
    staging.clear();
    for (lo, hi) in &cols {
        staging.extend_from_slice(&lo.to_le_bytes());
        staging.extend_from_slice(&hi.to_le_bytes());
    }
    debug_assert_eq!(staging.len(), columns * COLUMN_BYTES);
    let at = open.create(session, path).await?;
    open.files[at]
        .1
        .write_at(0, staging)
        .map_err(|text| Failed::Unstored(format!("an overview could not be written: {text}")))
}

#[cfg(test)]
pub(crate) mod tests;
