//! Stem explorer: waveform lanes for a song and its stems, with mute, solo,
//! volume, tempo, a loop region and click-to-seek.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use engine::mixdown::MixRecipe;
use engine::overview::lane_columns;
use engine::{Player, PlayerConfig, SharedParams, Song, Stem, StretcherKind};
use export::MixFollower;
use lanes::{LaneAudio, Shown, ShownLane};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

pub mod assemble;
pub mod decoder;
pub mod export;
pub mod lane_store;
pub mod lanes;
#[cfg(target_arch = "wasm32")]
pub mod picker;
#[cfg(target_arch = "wasm32")]
pub mod playback;
pub mod reset;
#[cfg(target_arch = "wasm32")]
pub mod separator;
pub mod spectrogram;
#[cfg(target_arch = "wasm32")]
pub mod store;
pub mod store_meta;
#[cfg(target_arch = "wasm32")]
pub mod streamtest;

/// Lanes that are not parts of the mix: computed from the others, or the
/// original recording kept beside its separated stems.  They start muted,
/// so the mix lane leaves them out until the user unmutes one.
pub const DERIVED: [&str; 2] = ["classic drums", "original"];

/// The mix of the original sources: every stem except derived ones, summed
/// with `dsp::mix`.
pub fn mix_of_sources(song: &Song) -> (Vec<f32>, Vec<f32>) {
    let src: Vec<&Stem> = song
        .stems
        .iter()
        .filter(|s| !DERIVED.contains(&s.name.as_str()))
        .collect();
    if src.is_empty() {
        return (vec![0.0; song.frames], vec![0.0; song.frames]);
    }
    let volumes = vec![1.0; src.len()];
    let lefts: Vec<&[f32]> = src.iter().map(|s| s.left.as_slice()).collect();
    let rights: Vec<&[f32]> = src.iter().map(|s| s.right.as_slice()).collect();
    (dsp::mix(&lefts, &volumes), dsp::mix(&rights, &volumes))
}

/// The lanes of `song` that are not derived, as a song that shares their
/// samples (MEM-1): the input of `mix_of_sources` and of classic drums.
/// `None` when one of them has no samples in memory.
fn sources_in_memory(song: &Shown) -> Option<Song> {
    let stems = song
        .lanes
        .iter()
        .filter(|lane| !DERIVED.contains(&lane.name.as_str()))
        .map(|lane| {
            let (left, right) = lane.audio.samples()?;
            Some(Stem {
                name: lane.name.clone(),
                left: left.clone(),
                right: right.clone(),
            })
        })
        .collect::<Option<Vec<Stem>>>()?;
    Some(Song {
        sample_rate: song.sample_rate,
        frames: song.frames,
        stems,
    })
}

/// The memory path's input of `song` (D9 of the M7 plan): the lane that
/// holds the original recording, as it is, else the mix of its sources at
/// volume 1, clamped, as `mix_of_sources` makes it.  It reads `audio`, the
/// lanes as a job reads them, and says whether it reads browser storage.
pub fn separation_input(
    song: &Shown,
    audio: Vec<LaneAudio>,
) -> (Box<dyn engine::mixdown::Pcm>, bool) {
    let (frames, rate) = (song.frames, song.sample_rate);
    match original_index_of(&song.names()) {
        Some(i) => {
            let lane = audio[i].clone();
            let stored = lane.samples().is_none();
            (Box::new(lanes::LanePcm::new(lane, frames, rate)), stored)
        }
        None => {
            let gains: Vec<f32> = song
                .lanes
                .iter()
                .map(|lane| {
                    if DERIVED.contains(&lane.name.as_str()) {
                        0.0
                    } else {
                        1.0
                    }
                })
                .collect();
            let stored = audio
                .iter()
                .zip(&gains)
                .any(|(lane, gain)| *gain > 0.0 && lane.samples().is_none());
            let recipe = MixRecipe::new(gains);
            (
                Box::new(lanes::LanesMix::new(&audio, frames, rate, &recipe)),
                stored,
            )
        }
    }
}

/// Whether the memory path's input of `song` is the session file at
/// `mix_path` (D10 of the M7 plan): then the session's mix is kept, and
/// `write_mix` never reads the file that it writes.
pub fn input_is_the_mix_file(song: &Shown, mix_path: &str) -> bool {
    original_index_of(&song.names())
        .and_then(|i| song.lanes[i].audio.file())
        .is_some_and(|file| file.path == mix_path)
}

/// How classic drums of a song is made (D8 of the M7 plan).
#[derive(Debug, PartialEq)]
pub enum ClassicRoute {
    /// Every input lane has a session file: the decode worker reads them,
    /// and writes a stored lane.
    Worker(Vec<store_meta::LaneFile>),
    /// Every input lane is in memory: the page runs the block HPSS.
    Memory,
    /// Some input lanes have a file and some are in memory, and the song
    /// fits the memory path: the page reads the stored ones first.
    ReadFirst,
    /// As `ReadFirst`, but the song is longer than the memory path takes.
    TooLong,
}

/// The route of classic drums of `song`, whose input is its lanes that are
/// not derived (D8 of the M7 plan).  `memory_limit` is the longest song
/// that the memory path takes.
pub fn classic_route(song: &Shown, memory_limit: usize) -> ClassicRoute {
    let files: Vec<Option<&store_meta::LaneFile>> = song
        .lanes
        .iter()
        .filter(|lane| !DERIVED.contains(&lane.name.as_str()))
        .map(|lane| lane.audio.file())
        .collect();
    if files.iter().all(Option::is_none) {
        ClassicRoute::Memory
    } else if files.iter().all(Option::is_some) {
        ClassicRoute::Worker(files.into_iter().flatten().cloned().collect())
    } else if song.frames <= memory_limit {
        ClassicRoute::ReadFirst
    } else {
        ClassicRoute::TooLong
    }
}

/// The audio of `song`'s lanes for a job: the lanes with a file read it
/// through `store`, when there is one (D3 and D1 of the M7 plan).
fn job_audio(
    song: &Shown,
    store: &Option<std::rc::Rc<dyn lane_store::LaneStore>>,
) -> Vec<LaneAudio> {
    match store {
        Some(store) => lanes::reading_files(song.audio(), store),
        None => song.audio(),
    }
}

/// One source for each lane of `song`, for the lanes that the read-ahead
/// buffer does not play: a lane in memory shares its samples (MEM-1), and
/// a stored lane plays silence; it never falls back to samples (D2 of the
/// M7 plan).
fn memory_or_silent_sources(song: &Shown) -> Vec<Arc<dyn engine::source::StemSource>> {
    song.lanes
        .iter()
        .map(|lane| -> Arc<dyn engine::source::StemSource> {
            match lane.audio.samples() {
                Some((left, right)) => Arc::new(engine::source::MemorySource::new(
                    left.clone(),
                    right.clone(),
                )),
                None => Arc::new(engine::source::SilentSource),
            }
        })
        .collect()
}

/// The lane that already holds a song's original recording: its "original"
/// lane, else its only source stem (an opened song file is one stem).
/// `None` when the original is a mix of several source stems.
///
/// A lone stem is taken as it is, not through `dsp::mix`, so it is not
/// clamped to +-1 and not copied: a copy of a song-sized buffer would stay in
/// the web page's memory for good.
fn original_lane(song: &Song) -> Option<&Stem> {
    original_index(song).map(|i| &song.stems[i])
}

/// The index of [`original_lane`] in `song.stems`.
fn original_index(song: &Song) -> Option<usize> {
    let names: Vec<&str> = song.stems.iter().map(|s| s.name.as_str()).collect();
    original_index_of(&names)
}

/// The index of the lane that holds the original recording, among lanes
/// of these `names`: "original", else the only source lane.
fn original_index_of(names: &[&str]) -> Option<usize> {
    if let Some(i) = names.iter().position(|n| *n == "original") {
        return Some(i);
    }
    let mut sources = names
        .iter()
        .enumerate()
        .filter(|(_, name)| !DERIVED.contains(name));
    match (sources.next(), sources.next()) {
        (Some((only, _)), None) => Some(only),
        _ => None,
    }
}

/// A separated song: the separated stems, then the source's original
/// recording as an "original" lane, shared with the source when it has one
/// lane holding it (see `original_lane`) and mixed from its stems otherwise.
pub fn with_original(source: &Song, mut stems: Vec<Stem>) -> Song {
    let original = match original_lane(source) {
        Some(lane) => Stem {
            name: "original".into(),
            left: lane.left.clone(),
            right: lane.right.clone(),
        },
        None => {
            let (left, right) = mix_of_sources(source);
            Stem {
                name: "original".into(),
                left: left.into(),
                right: right.into(),
            }
        }
    };
    stems.push(original);
    Song {
        sample_rate: source.sample_rate,
        frames: source.frames,
        stems,
    }
}

/// The lane file of each lane of a song, in the order of its stems: `None`
/// for a lane that plays from memory (D8 of M6).
pub type LaneFiles = Vec<Option<store_meta::LaneFile>>;

/// [`with_original`] of a shown song: the stems, each with its file in
/// `stem_files`, then the "original" lane.  It takes the audio of the
/// source lane that holds the original, with its file, or the mix of the
/// sources, with `mix`, the session's usable mix file (D8 of M6).  A song
/// of several sources that are not all in memory gets no "original" lane
/// (D1 of the M7 plan; step 7 gives it the session's mix).
pub fn with_original_files(
    source: &Shown,
    stems: Vec<Stem>,
    stem_files: LaneFiles,
    mix: Option<store_meta::LaneFile>,
) -> Shown {
    assert_eq!(stem_files.len(), stems.len(), "a lane file for each stem");
    let original = match original_index_of(&source.names()) {
        Some(i) => Some(source.lanes[i].audio.clone()),
        None => sources_in_memory(source).map(|sources| {
            let (left, right) = mix_of_sources(&sources);
            LaneAudio::with_file(left.into(), right.into(), mix)
        }),
    };
    let song = Song {
        sample_rate: source.sample_rate,
        frames: source.frames,
        stems,
    };
    let mut shown = Shown::with_files(song, stem_files);
    if let Some(audio) = original {
        shown.lanes.push(ShownLane {
            name: "original".into(),
            audio,
        });
    }
    shown
}

/// Results of browser work (reading files, browser storage) for the UI
/// thread to apply on its next frame.  Web only.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct Inbox {
    song: Option<anyhow::Result<Opened>>,
    /// The token of a Storage open that has finished, with or without a song.
    open_done: Option<u64>,
    status: Option<String>,
    /// Bytes used and available in browser storage.
    usage: Option<(f64, f64)>,
    /// Songs kept in browser storage.
    sessions: Option<Vec<(String, store_meta::Meta)>>,
    /// A reset that stopped short of reloading the page: its mode and why.
    reset_failed: Option<(reset::Reset, String)>,
    /// What a separation that is starting needs to choose its path: the
    /// serial of the song it was started on, the free bytes, and the frames
    /// of the session's usable mix file (D1 of M4).
    separation_start: Option<(u64, f64, Option<usize>)>,
    /// The mix write of a starting separation ended (D3 of M4): the serial
    /// of the song, and why the write failed, if it did.
    mix_written: Option<(u64, Result<(), String>)>,
    /// A storage-path separation whose stems are in place.
    stored: Option<StoredSeparation>,
    /// The memory path's input, read from browser storage (D9 of the M7
    /// plan).
    memory_input: Option<MemoryInput>,
    /// Classic drums of the song `serial`, made in the decode worker or
    /// from a read of browser storage (D8 of the M7 plan).
    classic: Option<ClassicMade>,
}

/// Classic drums that arrived for the song `serial` and the lanes of
/// `generation`.  Web only.
#[cfg(target_arch = "wasm32")]
struct ClassicMade {
    serial: u64,
    generation: u64,
    made: Result<ClassicLane, String>,
    /// Protects the session until the lane shows (D10 of M6).
    _open: Option<store::OpenGuard>,
}

/// The lane of classic drums: a stored WAV with its overview, or samples.
#[cfg(target_arch = "wasm32")]
enum ClassicLane {
    Stored((store_meta::LaneFile, lanes::Overview)),
    Memory((Vec<f32>, Vec<f32>)),
}

/// The memory path's input of the song `serial`, read from browser
/// storage into two JS arrays, for the separation started on `stage` with
/// `reason`.  Web only.
#[cfg(target_arch = "wasm32")]
struct MemoryInput {
    serial: u64,
    stage: JobStage,
    reason: Option<String>,
    read: Result<export::JsChannels, String>,
}

/// A storage-path separation after [`store::keep_stored_stems`].  Web only.
#[cfg(target_arch = "wasm32")]
struct StoredSeparation {
    job: SeparationJob,
    /// The stems read back, or `None` when its song is neither shown nor
    /// being opened.
    /// Each with its lane file (D8 of M6).
    stems: Option<anyhow::Result<(Vec<Stem>, LaneFiles)>>,
    /// The session's usable mix, for the "original" of several sources.
    mix: Option<store_meta::LaneFile>,
    secs: f64,
    /// The number of stems.
    count: usize,
    /// Why `meta` does not name the stems, if it does not.
    not_kept: Option<String>,
    /// The overviews of the stems and of their mix (D7 of the M7 plan).
    overviews: Option<store::StemOverviewSet>,
    /// Protects the session until the stems are shown (D10 of M6).
    _guard: Option<store::OpenGuard>,
}

/// A song to show, with what belongs to it, applied together so the shown
/// song never takes another song's session or name.  Web only.
#[cfg(target_arch = "wasm32")]
struct Opened {
    /// The song, with the lane file of each of its lanes (D8 of M6).
    song: Shown,
    /// The session's usable mix, for the "original" of several sources.
    mix: Option<store_meta::LaneFile>,
    /// The browser-storage session it is kept in.
    session: Option<String>,
    /// Its name, for downloads and messages.
    name: Option<String>,
    /// What the status line says after the song opens, if not the usual.
    note: Option<String>,
    /// Protects the session until the song is shown (D10 of M6).
    guard: Option<store::OpenGuard>,
}

#[cfg(target_arch = "wasm32")]
impl Opened {
    /// A song that plays from memory only.
    fn in_memory(song: Song, session: Option<String>, name: Option<String>) -> Self {
        Self {
            song: Shown::in_memory(song),
            mix: None,
            session,
            name,
            note: None,
            guard: None,
        }
    }
}

/// Where the Reset dialog is.  Web only.
#[cfg(target_arch = "wasm32")]
#[derive(Default, PartialEq)]
enum ResetStage {
    #[default]
    Closed,
    Asking,
    /// A choice was made; the page reloads when it is done.
    Running,
    /// The mode chosen, kept so Try again repeats it, and why it stopped.
    Failed(reset::Reset, String),
}

/// Asks the browser for the storage used and the stored songs, for the
/// Storage menu.
#[cfg(target_arch = "wasm32")]
async fn refresh_storage(pending: &std::rc::Rc<std::cell::RefCell<Inbox>>) {
    let usage = store::usage().await;
    let sessions = store::list().await;
    let mut inbox = pending.borrow_mut();
    inbox.usage = Some(usage);
    inbox.sessions = Some(sessions);
}

const SOURCE_URL: &str = "https://github.com/lloydmeta/stemyla";

/// The id of the published build this is, set by `xtask publish`: its
/// commit's 12-character hash, plus `-dirty-<unix seconds>` when it was
/// built with uncommitted changes.  `None` for any other build.
pub const BUILD_ID: Option<&str> = option_env!("STEMYLA_BUILD_ID");

/// The About menu's version line for a build id, and the commit it links
/// to when the id starts with a 12-character hash.
pub fn build_version(id: Option<&str>) -> (String, Option<String>) {
    let Some(id) = id else {
        return ("Version: unpublished build".into(), None);
    };
    let commit = id
        .get(..12)
        .filter(|hash| hash.bytes().all(|b| b.is_ascii_hexdigit()));
    (
        format!("Version: {id}"),
        commit.map(|hash| format!("{SOURCE_URL}/commit/{hash}")),
    )
}

/// The app's source, where the model comes from and the build's version,
/// with links: shown in the transport bar.
#[cfg(target_arch = "wasm32")]
pub fn about_menu(ui: &mut egui::Ui) {
    ui.menu_button("About", |ui| {
        ui.set_max_width(420.0);
        ui.hyperlink_to("Source code (GitHub): lloydmeta/stemyla", SOURCE_URL);
        ui.separator();
        ui.label(
            "Stems are separated by BS-RoFormer SW, a six-stem Band-Split RoPE Transformer, running on this computer's GPU.",
        );
        ui.hyperlink_to(
            "Model file (Hugging Face): lumabeat/bs-roformer-sw",
            "https://huggingface.co/lumabeat/bs-roformer-sw",
        );
        ui.hyperlink_to(
            "Model architecture (code, MIT): lucidrains/BS-RoFormer",
            "https://github.com/lucidrains/BS-RoFormer",
        );
        ui.hyperlink_to(
            "Paper: Music Source Separation with Band-Split RoPE Transformer",
            "https://arxiv.org/abs/2309.02612",
        );
        ui.label(
            "The trained weights have an unknown trainer and no stated licence, so this app does not host them; \
             \"Download model\" fetches the Hugging Face copy above.",
        );
        ui.hyperlink_to(
            "The open question about their licence",
            "https://github.com/openmirlab/bs-roformer-infer/issues/3",
        );
        ui.separator();
        match build_version(BUILD_ID) {
            (label, Some(commit)) => ui.hyperlink_to(label, commit),
            (label, None) => ui.weak(label),
        };
    });
}

#[cfg(not(target_arch = "wasm32"))]
const WELCOME: &str = "Drop a folder of stems (or several WAV/MP3/FLAC files), or press Demo song.";
#[cfg(target_arch = "wasm32")]
const WELCOME: &str = "Press Open song… (or drop a song, or several WAV/MP3/FLAC stems, on the page), or press Demo song.";

/// What the song picker offers: the formats `engine` decodes, as
/// extensions, since iOS Safari treats `audio/*` as video (WebKit bug
/// 242110).
#[cfg(target_arch = "wasm32")]
const SONG_FILES: &str = ".mp3,.wav,.flac";

/// The status line while the files open: "Opening "<name>"…".
#[cfg(target_arch = "wasm32")]
fn opening_text(chosen: &Chosen) -> String {
    let name = chosen
        .first()
        .map(|(n, _)| store_meta::file_title(n))
        .unwrap_or_default();
    format!("Opening \"{name}\"…")
}

/// The files that the user chose, as the browser gives them: each name and
/// its `File`, whose bytes the page does not read on the decode path.
#[cfg(target_arch = "wasm32")]
type Chosen = Vec<(String, web_sys::File)>;

/// Logs the page's WebAssembly memory at `when` (D12 of
/// `plans/active/stream-from-storage-m5.md`).
#[cfg(target_arch = "wasm32")]
fn log_page_memory(when: &str) {
    log::info!(
        "decoder: memory: page wasm {:.0} MB {when}",
        separator::page_wasm_mb()
    );
}

/// Opens the files that the user chose (D9): saves them to a new session
/// as the browser's files, reads nothing, and asks the decode worker for
/// the session files.  The page shows the song from them.  If the save
/// fails, or browser storage fails in the decode, the page reads the files
/// and decodes them itself, as before M5.  The new session becomes current
/// once the song opens; a song that does not open forgets it.  The result
/// arrives through the inbox.
#[cfg(target_arch = "wasm32")]
async fn open_chosen(
    chosen: Chosen,
    decoder: decoder::DecodeClient,
    pending: std::rc::Rc<std::cell::RefCell<Inbox>>,
    ctx: egui::Context,
) {
    let opened = open_chosen_now(&chosen, &decoder).await;
    pending.borrow_mut().song = Some(opened);
    ctx.request_repaint();
    refresh_storage(&pending).await;
}

#[cfg(target_arch = "wasm32")]
async fn open_chosen_now(
    chosen: &Chosen,
    decoder: &decoder::DecodeClient,
) -> anyhow::Result<Opened> {
    let name = chosen.first().map(|(n, _)| store_meta::file_title(n));
    let started = js_sys::Date::now();
    // D1: a drop with no source fails before any save or post.
    let any_source = chosen
        .iter()
        .any(|(n, _)| engine::song::is_stem(&engine::song::stem_name(n)));
    if !any_source {
        return Err(anyhow::anyhow!("no stems to load"));
    }
    let files: Vec<(String, web_sys::Blob)> = chosen
        .iter()
        .map(|(n, f)| (n.clone(), web_sys::Blob::from(f.clone())))
        .collect();
    let id = match store::save_sources(&files, engine::resample::SONG_RATE).await {
        Ok(id) => id,
        Err(e) if resetting(decoder) => {
            return Err(e.context("browser storage is being reset"));
        }
        Err(e) => {
            log::warn!("song not kept in browser storage: {e:#}");
            return open_on_the_page(chosen, None, name, started, None).await;
        }
    };
    let opened = open_saved(chosen, &id, name, started, decoder).await;
    // D9: the new session becomes current only once its song opens, so a
    // reload until then restores the song on screen.  D12: the "kept" line
    // comes only then, so an e2e check that reloads at it finds the song.
    match &opened {
        Ok(_) => {
            store::make_current(&id).await;
            log::info!("song kept in browser storage as session {id}");
        }
        // The reset removes every session.
        Err(_) if store::is_closed() => {}
        // The guard is dropped, so `forget` does not wait for it.
        Err(_) => {
            if !store::forget(&id).await {
                log::warn!("could not remove session {id} after a song that did not open");
            }
        }
    }
    opened
}

/// Whether a reset runs: the decode client or browser storage is closed.
/// An open or a restore then ends with an error; the page does not decode
/// the song itself (D9, finding 3-3 of M5).
#[cfg(target_arch = "wasm32")]
fn resetting(decoder: &decoder::DecodeClient) -> bool {
    decoder.is_closed() || store::is_closed()
}

/// Opens the chosen files, saved as session `id`, through the decode
/// worker, or on the page if browser storage fails.
#[cfg(target_arch = "wasm32")]
async fn open_saved(
    chosen: &Chosen,
    id: &str,
    name: Option<String>,
    started: f64,
    decoder: &decoder::DecodeClient,
) -> anyhow::Result<Opened> {
    let id = id.to_string();
    // D10 of M6: no failure removes a file of the session while the open
    // reads it, until the song is shown.
    let guard = store::OpenGuard::new(&id);
    let sources = match store::open_stored(&id).await {
        Some(Ok(store::StoredSong::NeedsDecode { sources, .. })) => sources,
        other => {
            let why = match other {
                Some(Err(e)) => format!("{e:#}"),
                _ => "the new session could not be read".into(),
            };
            if resetting(decoder) {
                return Err(anyhow::anyhow!("browser storage is being reset ({why})"));
            }
            return open_on_the_page(chosen, Some(id), name, started, Some(why)).await;
        }
    };
    match decode_session(&id, &sources, None, decoder).await {
        Decode::Song(song, files) => {
            log::info!(
                "song: opened at {} Hz in {:.2} s ({} frames)",
                song.sample_rate,
                (js_sys::Date::now() - started) / 1000.0,
                song.frames
            );
            Ok(Opened {
                song: Shown::with_files(song, files.into_iter().map(Some).collect()),
                mix: None,
                session: Some(id),
                name,
                note: None,
                guard: Some(guard),
            })
        }
        Decode::Undecodable(text) | Decode::Reset(text) => Err(anyhow::anyhow!(text)),
        Decode::OnThePage(why) if resetting(decoder) => {
            Err(anyhow::anyhow!("browser storage is being reset ({why})"))
        }
        Decode::OnThePage(why) => {
            open_on_the_page(chosen, Some(id), name, started, Some(why)).await
        }
    }
}

/// The memory path of D9: reads the chosen files and decodes them on the
/// page, as before M5.  `why`: the failure of browser storage that sent the
/// song here, for the status line.
#[cfg(target_arch = "wasm32")]
async fn open_on_the_page(
    chosen: &Chosen,
    session: Option<String>,
    name: Option<String>,
    started: f64,
    why: Option<String>,
) -> anyhow::Result<Opened> {
    log::info!("decoder: decoding on the page");
    let mut files = Vec::with_capacity(chosen.len());
    for (n, file) in chosen {
        let bytes = picker::read(file).await.map_err(|e| anyhow::anyhow!(e))?;
        files.push((n.clone(), bytes));
    }
    let song = Song::load_files(files)?;
    log::info!(
        "song: opened at {} Hz in {:.2} s ({} frames)",
        song.sample_rate,
        (js_sys::Date::now() - started) / 1000.0,
        song.frames
    );
    let note = why.map(|why| {
        log::warn!("decoder: browser storage failed: {why}");
        format!("Opened on this page: browser storage failed ({why}).")
    });
    Ok(Opened {
        note,
        ..Opened::in_memory(song, session, name)
    })
}

/// How a decode of a session by the decode worker ended, for the page.
#[cfg(target_arch = "wasm32")]
enum Decode {
    /// The session is complete, and its song read back (D7, D10), with
    /// the lane file of each source (D8 of M6).
    Song(Song, Vec<store_meta::LaneFile>),
    /// A source cannot be decoded: the text says why.
    Undecodable(String),
    /// Browser storage failed, or the worker did not answer: the page
    /// decodes the song itself (D9).  The text says why.
    OnThePage(String),
    /// A reset stopped the decode: the open or restore ends with this
    /// error, and the page decodes nothing.
    Reset(String),
}

/// D9: asks the decode worker to decode session `id` from `sources`,
/// keeping a usable mix of `mix_frames` (D8).  It removes the `decoded`
/// line first (D2).  The decode's guard holds the session from the post to
/// the end of D7, and is dropped before this returns.
#[cfg(target_arch = "wasm32")]
async fn decode_session(
    id: &str,
    sources: &[store_meta::PlannedSource],
    mix_frames: Option<usize>,
    decoder: &decoder::DecodeClient,
) -> Decode {
    if let Err(e) = store::clear_decoded(id).await {
        return Decode::OnThePage(format!("{e:#}"));
    }
    let request = stem_protocol::decode::DecodeRequest {
        session: store::session_folder(id),
        sources: sources
            .iter()
            .map(|s| stem_protocol::decode::Source {
                file: s.file.clone(),
                index: s.index,
                ext: s.ext.clone(),
                in_mix: s.in_mix,
            })
            .collect(),
        mix_frames: mix_frames.map(|f| f as u64),
    };
    let ended = decoder.decode(&request, store::DecodeGuard::new(id)).await;
    use decoder::Ended;
    use stem_protocol::decode::DecodeReply;
    match ended {
        Ended::Reply(
            DecodeReply::Decoded {
                frames,
                new_mix,
                channels,
            },
            guard,
        ) => {
            if channels.len() != sources.len() {
                return Decode::OnThePage(format!(
                    "the decode worker gave the channels of {} sources, not {}",
                    channels.len(),
                    sources.len()
                ));
            }
            let frames = frames as usize;
            let kept = store::keep_decoded(id, sources, frames, new_mix, &channels).await;
            drop(guard);
            log_page_memory("as the decode ends");
            match kept {
                Ok(store::Kept::Complete) => {
                    let decoded = store_meta::Decoded {
                        frames,
                        format: store_meta::MIX_FORMAT.into(),
                        channels: sources.iter().map(|s| s.index).zip(channels).collect(),
                    };
                    match store::read_back(id, sources, &decoded).await {
                        Ok((song, files)) => Decode::Song(song, files),
                        Err(e) => Decode::OnThePage(format!("{e:#}")),
                    }
                }
                Ok(store::Kept::MixBusy) => {
                    Decode::OnThePage("a separation of this song uses its mix file now".into())
                }
                Err(e) => Decode::OnThePage(format!("{e:#}")),
            }
        }
        Ended::Reply(DecodeReply::Undecodable { text }, guard) => {
            drop(guard);
            Decode::Undecodable(text)
        }
        Ended::Reply(DecodeReply::Unstored { text }, guard) => {
            drop(guard);
            // D8: a kept mix of other frames goes, so the next open
            // decodes again, unless a separation uses it.
            if mix_frames.is_some() && !store::mix_in_use(id) {
                let removal = store::forget_mix(id);
                if let Err(e) = removal.await {
                    log::warn!("decoder: could not remove the mix of session {id}: {e:#}");
                }
            }
            Decode::OnThePage(text)
        }
        Ended::Reply(other, guard) => {
            drop(guard);
            Decode::OnThePage(format!("the decode worker replied {}", other.kind()))
        }
        Ended::Stopped(reason) if resetting(decoder) => {
            Decode::Reset(format!("browser storage is being reset ({reason})"))
        }
        Ended::Stopped(reason) => Decode::OnThePage(format!("the decode worker stopped: {reason}")),
    }
}

/// D11: session `id` as the page opens it, made current: read back if it
/// is complete, else decoded by the decode worker first.  If browser
/// storage fails, the page decodes the stored sources itself
/// (`store::restore_id`).  `None` when the session has no `meta`.  The
/// text, if any, says why the song was decoded on the page.  The song comes
/// with the lane file of each lane that plays from storage (D8 of M6), and
/// the guard that protects the session until it is shown (D10 of M6).
#[cfg(target_arch = "wasm32")]
async fn restore_stored(
    id: &str,
    decoder: &decoder::DecodeClient,
) -> Option<anyhow::Result<(Opened, store_meta::Meta, Option<String>)>> {
    let guard = store::OpenGuard::new(id);
    let stored = match store::open_stored(id).await? {
        Ok(stored) => stored,
        Err(e) => return Some(Err(e)),
    };
    let (meta, sources, read_back) = match stored {
        store::StoredSong::Complete {
            meta,
            sources,
            decoded,
        } => {
            log::info!("decoder: read back a complete session");
            match store::read_back(id, &sources, &decoded).await {
                Ok(read) => (meta, sources, Some(read)),
                Err(e) => {
                    // D10: decode again.
                    log::warn!("decoder: could not read session {id} back: {e:#}");
                    (meta, sources, None)
                }
            }
        }
        store::StoredSong::NeedsDecode { meta, sources, .. } => (meta, sources, None),
    };
    let (song, files) = match read_back {
        Some(read) => read,
        None => {
            let mix_frames = store::usable_mix_frames(id, &meta).await;
            match decode_session(id, &sources, mix_frames, decoder).await {
                Decode::Song(song, files) => (song, files),
                Decode::Undecodable(text) | Decode::Reset(text) => {
                    return Some(Err(anyhow::anyhow!(text)));
                }
                Decode::OnThePage(_) if resetting(decoder) => {
                    return Some(Err(anyhow::anyhow!("browser storage is being reset")));
                }
                Decode::OnThePage(why) => {
                    log::info!("decoder: decoding on the page");
                    log::warn!("decoder: browser storage failed: {why}");
                    let rebuilt = store::restore_id(id).await?;
                    return Some(rebuilt.map(|(_, song, meta)| {
                        let note = format!("decoded on this page: browser storage failed ({why})");
                        let opened = Opened {
                            guard: Some(guard),
                            ..Opened::in_memory(song, Some(id.to_string()), Some(meta.name.clone()))
                        };
                        (opened, meta, Some(note))
                    }));
                }
            }
        }
    };
    let files: LaneFiles = files.into_iter().map(Some).collect();
    let mix = store::mix_lane(id, &meta, song.frames).await;
    let song = Shown::with_files(song, files);
    let song = if meta.stems.is_empty() {
        song
    } else {
        // D7 of the M7 plan: the overviews of the stems, made once.
        let overviews = store::stem_overviews(id, song.frames, decoder).await;
        match store::stems_of(id, &meta, song.frames).await {
            Ok((stems, stem_files)) => {
                let mut song = with_original_files(&song, stems, stem_files, mix.clone());
                song.mix_overview = overviews.map(|o| o.mix);
                song
            }
            Err(e) => return Some(Err(e)),
        }
    };
    store::make_current(id).await;
    let opened = Opened {
        song,
        mix,
        session: Some(id.to_string()),
        name: Some(meta.name.clone()),
        note: None,
        guard: Some(guard),
    };
    Some(Ok((opened, meta, None)))
}

/// Milliseconds of each frame that the mix lane's waveform job may take.
const PEAKS_MS: f64 = 3.0;
/// Milliseconds of each frame that the spectrogram jobs may take.
const SPECTROGRAM_MS: f64 = 4.0;
/// Milliseconds of each frame that a save may take.  The page's main
/// thread also fills the Web Audio buffers.
#[cfg(target_arch = "wasm32")]
const SAVE_MS: f64 = 4.0;
const LANE_HEIGHT: f32 = 64.0;
const LOOP_BAR_HEIGHT: f32 = 18.0;

/// One waveform lane.
pub struct Lane {
    pub name: String,
    /// `None` for the mix lane.
    pub stem: Option<usize>,
    pub color: Color32,
    /// (min, max) per column, from `dsp::peaks`.
    pub columns: Vec<(f32, f32)>,
    /// 1 / loudest absolute peak (from `dsp::biggest`), so every lane fills
    /// its height.
    pub scale: f32,
    pub spectrogram: Option<egui::TextureHandle>,
}

/// Where audio goes.
pub enum Output {
    /// A real device through cpal, opened once and kept while songs come
    /// and go at its sample rate; the player is in its slot.
    #[cfg(feature = "audio")]
    Live(engine::live::LiveOutput),
    /// No device: the UI renders the player silently so the playhead still
    /// moves.  Used in tests, in headless runs, and if the device fails.
    Silent(Box<Player>),
}

pub struct StemApp {
    /// The shown song: the audio of each lane (D1 of the M7 plan).
    pub song: Option<std::rc::Rc<Shown>>,
    /// Counts the songs opened (not lanes added to the shown one), so work
    /// that finishes later can tell whether its song is still the one shown.
    song_serial: u64,
    /// Counts the lanes shown: each `load_song` adds one.  Work that ends
    /// later for the shown song is applied only to the lanes it started on
    /// (D1 of the M7 plan).
    lanes_generation: u64,
    pub params: Option<Arc<SharedParams>>,
    pub lanes: Vec<Lane>,
    /// Screen rectangles of the lanes drawn in the last frame (mix first).
    pub lane_rects: Vec<Rect>,
    pub loop_bar_rect: Option<Rect>,
    pub status: String,
    pub output: Option<Output>,
    /// Keeps the mix lane on the mix as heard.
    mix_follower: Option<MixFollower>,
    /// Makes the lanes' spectrograms, while they are on.
    spectrograms: spectrogram::Spectrograms,
    /// Where the jobs read the lanes that have a file (D3 of the M7 plan):
    /// browser storage on the web, none natively, where a song is in
    /// memory.
    pub lane_store: Option<std::rc::Rc<dyn lane_store::LaneStore>>,
    pub tempo_pct: f32,
    pub show_spectrograms: bool,
    /// Ask for a real audio device when an output is opened: for the first
    /// song, and for a later one the kept output cannot play.
    pub want_device: bool,
    stretcher: StretcherKind,
    loop_drag_anchor: Option<u64>,
    silent_scratch: (Vec<f32>, Vec<f32>),
    /// Seconds until the next audio-cost log line.
    stats_timer: f32,
    /// (callbacks, total ns) at the previous log line.
    stats_prev: (u64, u64),
    /// A classic-drums job running on another thread (native only).
    #[cfg(not(target_arch = "wasm32"))]
    classic_job: Option<ClassicJob>,
    /// The song serial and lanes generation of classic drums that the page
    /// waits for (web only, D8 of the M7 plan).
    #[cfg(target_arch = "wasm32")]
    classic_request: Option<(u64, u64)>,
    #[cfg(target_arch = "wasm32")]
    pending: std::rc::Rc<std::cell::RefCell<Inbox>>,
    /// The read-ahead buffer and reader of the shown song's stored lanes.
    #[cfg(target_arch = "wasm32")]
    playback: Option<playback::Playback>,
    /// The page's egui context, for work that ends between frames.
    #[cfg(target_arch = "wasm32")]
    ctx: Option<egui::Context>,
    /// Browser-storage session of the shown song, if it is stored.
    #[cfg(target_arch = "wasm32")]
    session: Option<String>,
    /// Browser storage used and available (bytes), when last asked.
    #[cfg(target_arch = "wasm32")]
    usage: (f64, f64),
    /// The song's name (its file name), for downloads.
    #[cfg(target_arch = "wasm32")]
    song_name: Option<String>,
    /// Songs kept in browser storage (id, meta), newest first.
    #[cfg(target_arch = "wasm32")]
    sessions: Vec<(String, store_meta::Meta)>,
    /// `?streamtest`: the state of the storage playback test.
    #[cfg(target_arch = "wasm32")]
    streamtest: Option<std::rc::Rc<std::cell::RefCell<streamtest::Status>>>,
    /// The stem separation worker (web only).
    #[cfg(target_arch = "wasm32")]
    separator: Option<separator::Separator>,
    /// The decode worker's client (web only), which starts the worker at
    /// the first decode.
    #[cfg(target_arch = "wasm32")]
    decoder: decoder::DecodeClient,
    /// The song the running separation was started on.
    #[cfg(target_arch = "wasm32")]
    separating: Option<SeparationJob>,
    /// Stems of a storage-path separation are being moved and read; the
    /// Separate button waits for them (D6 of M3).
    #[cfg(target_arch = "wasm32")]
    reading_stems: bool,
    /// The longest song the memory path takes: [`MEMORY_PATH_MAX_FRAMES`],
    /// or less in a test.
    #[cfg(target_arch = "wasm32")]
    memory_limit: usize,
    /// The Storage menu's open in progress (one at a time): its token and
    /// the session it opens.
    #[cfg(target_arch = "wasm32")]
    opening: Option<(u64, String)>,
    /// Tokens handed to Storage opens so far.
    #[cfg(target_arch = "wasm32")]
    opens: u64,
    /// Stems that arrived while their song was being opened (and so may be
    /// missing from what the open read): the session, the stems, and what
    /// to say if the open does not show them.
    #[cfg(target_arch = "wasm32")]
    stems_for_opening: Option<(String, Vec<Stem>, String)>,
    #[cfg(target_arch = "wasm32")]
    reset: ResetStage,
    /// The save in progress, one at a time.
    #[cfg(target_arch = "wasm32")]
    saving: Option<Saving>,
    /// What to add to "Saved ..." when the worker's file arrives, such as
    /// that the mix clips.
    #[cfg(target_arch = "wasm32")]
    save_notes: export::SaveNotes,
}

/// A stem or a mix being made into a file (decision 0014).
#[cfg(target_arch = "wasm32")]
struct Saving {
    job: export::ExportJob<export::JsSink>,
    /// The file's name, with its extension.
    name: String,
    rate: u32,
    /// Whether it is the mix lane's save.
    mix: bool,
}

/// A classic-drums job: the serial of the song and the generation of the
/// lanes it started on, and where its lane arrives.
#[cfg(not(target_arch = "wasm32"))]
type ClassicJob = (u64, u64, std::sync::mpsc::Receiver<(Vec<f32>, Vec<f32>)>);

/// The song a separation was started on, so its stems go to that song even
/// if another is opened while it runs.
pub struct SeparationJob {
    serial: u64,
    /// The song's browser-storage session, where its stems are kept.
    session: Option<String>,
    name: Option<String>,
    stage: JobStage,
    /// False after browser storage refused the stems (D4 of M3): the
    /// memory path then does not save them.
    keeps_stems: bool,
}

/// How far a separation has got.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JobStage {
    /// Separate was pressed; the page waits for the browser's storage
    /// estimate before it chooses the path.
    Starting,
    /// The page writes the song's mix file to the session (D3 of M4)
    /// before it posts on the storage path.
    WritingMix,
    /// The worker writes the stems to the session's files.
    Storage,
    /// The worker sends the stems to the page.
    Memory,
}

/// The longest song that the memory path separates: 100 seconds at
/// 44.1 kHz, on every device, because a page cannot read its memory limit.
///
/// M2 of `plans/active/stream-from-storage.md` measured peaks of 966 MB for
/// a 1-minute song and 1558 MB for a 4-minute song, at `chunk=16384`.  So
/// the peak grows by about 197 MB a minute, from about 769 MB at 0
/// minutes.  With 950 MB for the parts that M2 does not measure, a song
/// fits in an iPhone's 2048 MB if 769 + 197 x minutes + 950 is at most
/// 2048: 1.67 minutes or less.  The default chunk holds about 250 MB more
/// on the GPU, and the 950 MB is a margin, so the two differences about
/// cancel.  The plan's estimate from the iPhone also gives about 1.7
/// minutes.  1.67 minutes rounds down to 100 seconds.  M10 can change
/// this on its measures.
pub const MEMORY_PATH_MAX_FRAMES: usize = 100 * 44_100;

/// Which way a separation goes (D2 of M3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeparationPath {
    /// The worker writes the stems to the song's session files.
    Storage,
    /// The worker sends the stems to the page, which puts them together.
    Memory,
    /// The song has no room in storage and is too long for the memory path.
    TooLong,
}

/// Bytes of browser storage that the six stems of a song of `frames`
/// need: six 16-bit stereo WAV files, and 16 MB to spare.
pub fn storage_needed(frames: usize) -> f64 {
    6.0 * (44.0 + 4.0 * frames as f64) + 16e6
}

/// Bytes of browser storage that the storage path needs for a song of
/// `frames`: its stems, and its mix file unless the session keeps a usable
/// one (`mix_kept`, D7 of M4).
pub fn storage_path_needs(frames: usize, mix_kept: bool) -> f64 {
    let mix = if mix_kept { 0.0 } else { 8.0 * frames as f64 };
    storage_needed(frames) + mix
}

/// The storage path if the song has a session and `free_bytes` holds
/// `needed_bytes` ([`storage_path_needs`]); else the memory path if the
/// song is at most `max_memory_frames` long; else none.
pub fn separation_path(
    has_session: bool,
    frames: usize,
    free_bytes: f64,
    needed_bytes: f64,
    max_memory_frames: usize,
) -> SeparationPath {
    if has_session && free_bytes >= needed_bytes {
        SeparationPath::Storage
    } else if frames <= max_memory_frames {
        SeparationPath::Memory
    } else {
        SeparationPath::TooLong
    }
}

/// Whether the page offers Separate: not while a separation starts
/// (`separating`, until the worker runs it) or while the page reads the
/// stems of a storage-path separation back (`reading_stems`, D6 of M3).
pub fn separate_enabled(separating: bool, reading_stems: bool) -> bool {
    !separating && !reading_stems
}

/// Whether the Storage menu offers "Forget" for session `id` while `job`
/// runs: not while a starting or storage-path separation may write into it
/// (D7 of M3), the page's mix write included.  Not for a `protected` session
/// either: the shown song plays from it, or an open reads it (D10 of M6).
pub fn forget_enabled(id: &str, job: Option<&SeparationJob>, protected: bool) -> bool {
    !protected
        && !job.is_some_and(|j| j.session.as_deref() == Some(id) && j.stage != JobStage::Memory)
}

/// What the page does after browser storage refused a separation's stems
/// (D4 of M3).
#[derive(Debug, PartialEq)]
pub enum AfterUnstored {
    /// Separate the shown song again, on the memory path.
    Memory,
    /// The shown song is too long for the memory path.
    TooLong,
    /// Another song is shown: the user opens the song and separates it
    /// again.
    OpenAgain,
}

/// [`AfterUnstored`] for a separation whose stems would go to
/// `destination`, of a shown song of `frames`.
pub fn after_unstored(
    destination: &StemsGo,
    frames: usize,
    max_memory_frames: usize,
) -> AfterUnstored {
    match destination {
        StemsGo::Show if frames <= max_memory_frames => AfterUnstored::Memory,
        StemsGo::Show => AfterUnstored::TooLong,
        StemsGo::Kept | StemsGo::Discarded => AfterUnstored::OpenAgain,
    }
}

/// Where a finished separation's stems go.
#[derive(Debug, PartialEq)]
pub enum StemsGo {
    /// Onto the shown song, which is the one separated.
    Show,
    /// Only into the separated song's browser storage.
    Kept,
    /// Nowhere: the song is neither shown nor kept.
    Discarded,
}

impl SeparationJob {
    /// The job's song is shown if it has stayed open since Separate, or
    /// has been opened again from its stored session.
    pub fn destination(&self, shown_serial: u64, shown_session: Option<&str>) -> StemsGo {
        let reopened = self.session.is_some() && self.session.as_deref() == shown_session;
        if self.serial == shown_serial || reopened {
            StemsGo::Show
        } else if self.session.is_some() {
            StemsGo::Kept
        } else {
            StemsGo::Discarded
        }
    }
}

/// The only sample rate the separation model takes, and so its stems' rate.
/// Every opened song is at this rate already (decision 0012).
#[cfg(target_arch = "wasm32")]
const SEPARATION_RATE: u32 = engine::resample::SONG_RATE;

impl Default for StemApp {
    fn default() -> Self {
        Self {
            song: None,
            song_serial: 0,
            lanes_generation: 0,
            params: None,
            lanes: Vec::new(),
            lane_rects: Vec::new(),
            loop_bar_rect: None,
            status: WELCOME.into(),
            output: None,
            mix_follower: None,
            spectrograms: spectrogram::Spectrograms::default(),
            #[cfg(target_arch = "wasm32")]
            lane_store: Some(std::rc::Rc::new(lane_store::OpfsStore::new(None))),
            #[cfg(not(target_arch = "wasm32"))]
            lane_store: None,
            tempo_pct: 100.0,
            show_spectrograms: false,
            want_device: true,
            stretcher: StretcherKind::default_pitch_preserving(),
            loop_drag_anchor: None,
            silent_scratch: (vec![0.0; 8192], vec![0.0; 8192]),
            stats_timer: 2.0,
            stats_prev: (0, 0),
            #[cfg(not(target_arch = "wasm32"))]
            classic_job: None,
            #[cfg(target_arch = "wasm32")]
            classic_request: None,
            #[cfg(target_arch = "wasm32")]
            pending: Default::default(),
            #[cfg(target_arch = "wasm32")]
            playback: None,
            #[cfg(target_arch = "wasm32")]
            ctx: None,
            #[cfg(target_arch = "wasm32")]
            session: None,
            #[cfg(target_arch = "wasm32")]
            usage: (0.0, 0.0),
            #[cfg(target_arch = "wasm32")]
            song_name: None,
            #[cfg(target_arch = "wasm32")]
            sessions: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            streamtest: None,
            #[cfg(target_arch = "wasm32")]
            separator: None,
            #[cfg(target_arch = "wasm32")]
            decoder: decoder::DecodeClient::new(decoder::LOADER),
            #[cfg(target_arch = "wasm32")]
            separating: None,
            #[cfg(target_arch = "wasm32")]
            reading_stems: false,
            #[cfg(target_arch = "wasm32")]
            memory_limit: MEMORY_PATH_MAX_FRAMES,
            #[cfg(target_arch = "wasm32")]
            opening: None,
            #[cfg(target_arch = "wasm32")]
            opens: 0,
            #[cfg(target_arch = "wasm32")]
            stems_for_opening: None,
            #[cfg(target_arch = "wasm32")]
            reset: ResetStage::Closed,
            #[cfg(target_arch = "wasm32")]
            saving: None,
            #[cfg(target_arch = "wasm32")]
            save_notes: export::SaveNotes::default(),
        }
    }
}

pub fn stem_color(name: &str) -> Color32 {
    match name {
        "drums" | "classic drums" | "percussive" => Color32::from_rgb(230, 90, 70),
        "bass" => Color32::from_rgb(90, 140, 230),
        "guitar" => Color32::from_rgb(240, 170, 60),
        "keys" | "piano" => Color32::from_rgb(120, 200, 120),
        "vocals" | "lead" => Color32::from_rgb(200, 110, 210),
        "other" | "harmonic" => Color32::from_rgb(90, 200, 200),
        _ => Color32::from_rgb(180, 180, 180),
    }
}

impl StemApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Same look everywhere (the browser would otherwise follow the OS).
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        #[allow(unused_mut)]
        let mut app = Self::default();
        #[cfg(target_arch = "wasm32")]
        {
            app.ctx = Some(cc.egui_ctx.clone());
            app.lane_store = Some(std::rc::Rc::new(lane_store::OpfsStore::new(Some(
                cc.egui_ctx.clone(),
            ))));
        }
        #[cfg(target_arch = "wasm32")]
        if streamtest::Settings::from_url().is_some() {
            app.streamtest = Some(Default::default());
        }
        #[cfg(target_arch = "wasm32")]
        match separator::Separator::new(cc.egui_ctx.clone()) {
            Ok(s) => app.separator = Some(s),
            Err(e) => log::warn!("stem separation unavailable: {e}"),
        }
        #[cfg(target_arch = "wasm32")]
        {
            // Bring back the song (and stems) kept from the last visit.
            let (pending, ctx) = (app.pending.clone(), cc.egui_ctx.clone());
            let decoder = app.decoder.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let restored = match store::current().await {
                    Some(id) => restore_stored(&id, &decoder).await.map(|r| (id, r)),
                    None => None,
                };
                if let Some((_, r)) = restored {
                    let mut inbox = pending.borrow_mut();
                    match r {
                        Ok((opened, meta, note)) => {
                            let what = if meta.stems.is_empty() {
                                ""
                            } else {
                                " and its separated stems"
                            };
                            let text =
                                format!("Restored \"{}\"{what} from this browser.", meta.name);
                            log::info!("{text}");
                            let text = match note {
                                Some(note) => format!("{text}  It was {note}."),
                                None => text,
                            };
                            inbox.status = Some(text);
                            inbox.song = Some(Ok(opened));
                        }
                        Err(e) => {
                            log::warn!("could not restore the stored song: {e:#}");
                            inbox.status =
                                Some(format!("Could not restore the stored song: {e:#}"));
                        }
                    }
                }
                refresh_storage(&pending).await;
                ctx.request_repaint();
            });
        }
        app
    }

    /// Shows a different song, with the lane file of each lane, `None` for
    /// a lane that plays from memory (D8 of M6).  [`Self::load_song`] alone
    /// is for the shown song with lanes added or replaced.
    pub fn open_song(&mut self, song: Shown) {
        self.song_serial += 1;
        self.load_song(song);
    }

    /// [`Self::open_song`] for a song whose lanes all play from memory.
    pub fn open_memory_song(&mut self, song: Song) {
        self.open_song(Shown::in_memory(song));
    }

    /// The lane file of each lane of the shown song (D8 of M6): `None` for
    /// a lane that plays from memory.
    pub fn lane_files(&self) -> LaneFiles {
        self.song.as_ref().map(|s| s.files()).unwrap_or_default()
    }

    /// The shown song, as the one a separation starting now belongs to.
    pub fn separation_job(&self) -> Option<SeparationJob> {
        self.song.as_ref()?;
        Some(SeparationJob {
            serial: self.song_serial,
            #[cfg(target_arch = "wasm32")]
            session: self.session.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            session: None,
            #[cfg(target_arch = "wasm32")]
            name: self.song_name.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            name: None,
            stage: JobStage::Starting,
            keeps_stems: true,
        })
    }

    /// Gives the stems of a memory-path separation to the song it was
    /// started on: kept in browser storage with that song, unless storage
    /// refused them before (D4 of M3), and shown if it is still the open
    /// song.
    pub fn apply_separated(&mut self, job: SeparationJob, stems: Vec<Stem>, secs: f64) {
        #[cfg(target_arch = "wasm32")]
        if let Some(id) = job.session.clone().filter(|_| job.keeps_stems) {
            let destination = job.destination(self.song_serial, self.session.as_deref());
            let being_opened = self
                .opening
                .as_ref()
                .is_some_and(|(_, opening)| *opening == id);
            let kept = (destination == StemsGo::Kept && !being_opened)
                .then(|| kept_text(&job, stems.len(), secs));
            let (to_save, pending) = (stems.clone(), self.pending.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let status = match store::save_stems(&id, SEPARATION_RATE, &to_save).await {
                    Ok(()) => kept,
                    Err(e) => Some(format!("Stems not kept in browser storage: {e:#}")),
                };
                if let Some(status) = status {
                    pending.borrow_mut().status = Some(status);
                }
                refresh_storage(&pending).await;
            });
        }
        let name = song_name(&job);
        let kept_status = format!(
            "Separated \"{name}\" into {} stems; keeping them in this browser…",
            stems.len()
        );
        let note =
            (!job.keeps_stems).then(|| "The stems are not kept in this browser.".to_string());
        let files = vec![None; stems.len()];
        self.show_separated(job, (stems, files), (None, None), secs, kept_status, note);
    }

    /// Shows separated stems on the song they belong to, if it is shown.
    /// Both paths end here.  `kept_status` is the status line when that
    /// song is not shown but kept, and `note` is added to the status line.
    fn show_separated(
        &mut self,
        job: SeparationJob,
        (stems, stem_files): (Vec<Stem>, LaneFiles),
        (mix, mix_overview): (
            Option<store_meta::LaneFile>,
            Option<std::rc::Rc<lanes::Overview>>,
        ),
        secs: f64,
        kept_status: String,
        note: Option<String>,
    ) {
        let n = stems.len();
        log::info!("separator: {n} stems in {secs:.1} s");
        #[cfg(target_arch = "wasm32")]
        let shown_session = self.session.as_deref();
        #[cfg(not(target_arch = "wasm32"))]
        let shown_session = None;
        let destination = job.destination(self.song_serial, shown_session);
        let name = song_name(&job);
        #[cfg(target_arch = "wasm32")]
        if destination == StemsGo::Kept
            && let Some(id) = job.session.clone()
            && self.opening.as_ref().is_some_and(|(_, o)| *o == id)
        {
            // The open may have read the session before these stems.  If
            // they are not kept, a failed open says so (finding 3-9).
            let kept = note.clone().unwrap_or_else(|| kept_text(&job, n, secs));
            self.stems_for_opening = Some((id, stems, kept));
            self.status = kept_status;
            return;
        }
        let song = match (destination, self.song.clone()) {
            (StemsGo::Show, Some(song)) => song,
            (StemsGo::Kept, _) => {
                self.status = kept_status;
                return;
            }
            (StemsGo::Show, None) => {
                self.status = format!(
                    "Separated \"{name}\" into {n} stems, with no song open to show them on."
                );
                return;
            }
            (StemsGo::Discarded, _) => {
                self.status = format!(
                    "Separated \"{name}\" into {n} stems, but discarded them: that song is no longer open and is not kept in this browser."
                );
                return;
            }
        };
        let head = self.params.as_ref().map(|p| p.playhead());
        let mut separated = with_original_files(&song, stems, stem_files, mix);
        // The stems' mix overview (D7 of the M7 plan).
        separated.mix_overview = mix_overview;
        self.load_song(separated);
        if let (Some(h), Some(p)) = (head, &self.params) {
            p.request_seek(h);
        }
        self.status = format!("Separated into {n} stems in {secs:.0} s.  {}", self.status);
        if let Some(note) = note {
            self.status = format!("{}  {note}", self.status);
        }
        #[cfg(target_arch = "wasm32")]
        log::info!(
            "separator: memory: page wasm {:.0} MB with the stems shown",
            separator::page_wasm_mb()
        );
    }

    /// The stems of a storage-path separation are in place, and read back
    /// for a song shown or being opened.
    #[cfg(target_arch = "wasm32")]
    fn stored_stems_arrived(&mut self, stored: StoredSeparation) {
        let StoredSeparation {
            job,
            stems,
            mix,
            secs,
            count,
            not_kept,
            overviews,
            _guard,
        } = stored;
        match stems {
            None => {
                self.status = not_kept.unwrap_or_else(|| kept_text(&job, count, secs));
            }
            Some(Err(e)) => {
                log::error!("separator: could not read the stored stems: {e:#}");
                self.status = format!("Separated, but could not read the stems back: {e:#}");
                if let Some(not_kept) = not_kept {
                    self.status = format!("{}.  {not_kept}", self.status);
                }
            }
            Some(Ok(stems)) => {
                let kept_status = not_kept
                    .clone()
                    .unwrap_or_else(|| kept_text(&job, stems.0.len(), secs));
                let mix_overview = overviews.map(|o| o.mix);
                self.show_separated(job, stems, (mix, mix_overview), secs, kept_status, not_kept);
            }
        }
    }

    /// Loads a song: builds lanes, shared params and the output.  A lane
    /// with a file plays from browser storage through the read-ahead
    /// buffer (D8 of M6).
    pub fn load_song(&mut self, song: Shown) {
        // The old lanes' jobs go before the new lanes are made.  The old
        // player and the old reader go first too, so two read-ahead
        // buffers never live at once (D9 of M6).
        self.mix_follower = None;
        self.spectrograms = spectrogram::Spectrograms::default();
        self.stop_output();
        #[cfg(target_arch = "wasm32")]
        {
            self.playback = None;
        }
        self.lanes_generation += 1;
        let song = std::rc::Rc::new(song);
        let mut lanes = Vec::with_capacity(song.lanes.len() + 1);
        // D4 of the M7 plan: the mix lane draws the mix of the sources in
        // memory, or the song's mix overview.  With neither, it starts
        // empty, and the follower makes it.
        let mix = match (sources_in_memory(&song), &song.mix_overview) {
            (Some(sources), _) => {
                let (ml, mr) = mix_of_sources(&sources);
                Some(lane_columns(&ml, &mr))
            }
            (None, Some(overview)) => Some((overview.columns.clone(), overview.scale)),
            (None, None) => None,
        };
        let mix_shown = mix.is_some();
        let (cols, scale) = mix.unwrap_or((Vec::new(), 1.0));
        lanes.push(Lane {
            name: "mix".into(),
            stem: None,
            color: Color32::from_rgb(220, 220, 220),
            columns: cols,
            scale,
            spectrogram: None,
        });
        for (i, lane) in song.lanes.iter().enumerate() {
            // Until step 7, a lane with samples and a file draws from its
            // samples; a stored lane draws from its overview (D4).
            let (cols, scale) = match &lane.audio {
                LaneAudio::Memory { left, right } | LaneAudio::ReadBack { left, right, .. } => {
                    lane_columns(left, right)
                }
                LaneAudio::Stored { overview, .. } => (overview.columns.clone(), overview.scale),
            };
            lanes.push(Lane {
                name: lane.name.clone(),
                stem: Some(i),
                color: stem_color(&lane.name),
                columns: cols,
                scale,
                spectrogram: None,
            });
        }
        self.lanes = lanes;
        let params = Arc::new(SharedParams::new(song.lanes.len()));
        params.set_tempo(self.tempo_pct / 100.0);
        for (i, lane) in song.lanes.iter().enumerate() {
            if DERIVED.contains(&lane.name.as_str()) {
                params.set_muted(i, true);
            }
        }
        // The mix lane shows every source stem at full volume, which is
        // what the new params play.  An empty mix lane shows no mix yet.
        self.mix_follower = Some(if mix_shown {
            MixFollower::new(MixRecipe::as_heard(&params))
        } else {
            MixFollower::unshown()
        });
        // A save holds its song; it does not outlive the song's lanes.
        #[cfg(target_arch = "wasm32")]
        {
            self.saving = None;
        }
        let sources = self.lane_sources(&song, &params);
        let player = Player::with_sources(
            song.sample_rate,
            song.frames,
            Arc::clone(&params),
            self.stretcher.build(song.sample_rate),
            PlayerConfig::default(),
            sources,
        );
        self.status = format!(
            "{} stems, {:.1} s at {} Hz",
            song.lanes.len(),
            song.duration_s(),
            song.sample_rate
        );
        self.song = Some(song);
        self.params = Some(params);
        self.play_on_output(player);
    }

    /// One source for each lane of `song`: a lane with a file plays from
    /// browser storage through a new read-ahead buffer, made before the
    /// player, and the others from memory, sharing their samples (MEM-1).
    /// Records the session that the song plays from (D10 of M6): it stays
    /// protected while a lane of the song has a file, also when no
    /// read-ahead buffer is made (D1 of the M7 plan).
    fn lane_sources(
        &mut self,
        song: &Shown,
        params: &Arc<SharedParams>,
    ) -> Vec<Arc<dyn engine::source::StemSource>> {
        let memory = memory_or_silent_sources(song);
        #[cfg(target_arch = "wasm32")]
        {
            let names: Vec<String> = song.lanes.iter().map(|l| l.name.clone()).collect();
            let started = playback::Playback::start(
                &song.files(),
                &names,
                song.frames,
                Arc::clone(params),
                self.pending.clone(),
                self.ctx.clone(),
            );
            let Some((playback, stored)) = started else {
                let protected = song.has_stored_lane().then(|| self.session.clone());
                store::set_playing(protected.flatten());
                return memory;
            };
            log::info!(
                "playback: {} of {} lanes play from browser storage",
                playback.stored,
                playback.lanes
            );
            self.playback = Some(playback);
            store::set_playing(self.session.clone());
            stored
                .into_iter()
                .zip(memory)
                .map(|(stored, memory)| stored.unwrap_or(memory))
                .collect()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = params;
            memory
        }
    }

    /// Frees the playing player before the next one is built (the old song
    /// goes when `self.song` is replaced).  A live output stays open,
    /// silent, for the next song.
    fn stop_output(&mut self) {
        match &self.output {
            #[cfg(feature = "audio")]
            Some(Output::Live(live)) => drop(live.stop()),
            _ => self.output = None,
        }
    }

    /// Plays `player` on the live output if one is open and can play it
    /// (`engine::live::Unusable`); otherwise opens an output for it.
    fn play_on_output(&mut self, player: Player) {
        #[cfg(feature = "audio")]
        let player = match self.play_on_live(player) {
            Ok(()) => return,
            Err(player) => *player,
        };
        self.output = None;
        self.output = Some(self.open_output(player));
    }

    /// Swaps `player` into the open live output, freeing the one it
    /// replaces, or hands it back when there is no output it can play on.
    #[cfg(feature = "audio")]
    fn play_on_live(&mut self, player: Player) -> Result<(), Box<Player>> {
        let Some(Output::Live(live)) = &self.output else {
            return Err(Box::new(player));
        };
        let previous = live.play(player).map_err(|(why, player)| {
            log::info!("audio: opening a new output ({why:?})");
            player
        })?;
        drop(previous);
        self.status
            .push_str(&format!(", playing on {}", live.device_name));
        Ok(())
    }

    fn open_output(&mut self, player: Player) -> Output {
        #[cfg(feature = "audio")]
        if self.want_device {
            match engine::live::probe_default_output() {
                Ok(_) => match engine::live::start(player) {
                    Ok(out) => {
                        self.status
                            .push_str(&format!(", playing on {}", out.device_name));
                        return Output::Live(out);
                    }
                    Err((e, player)) => {
                        self.status.push_str(&format!(", audio failed: {e:#}"));
                        return Output::Silent(player);
                    }
                },
                Err(e) => self
                    .status
                    .push_str(&format!(", no audio device ({e:#}); silent")),
            }
        }
        Output::Silent(Box::new(player))
    }

    /// Loads stems from in-memory files (a multi-file drop).
    pub fn load_files(&mut self, files: Vec<(String, Vec<u8>)>) {
        match Song::load_files(files) {
            Ok(s) => self.open_memory_song(s),
            Err(e) => self.status = format!("Could not load: {e:#}"),
        }
    }

    pub fn load_demo(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            self.session = None;
            self.song_name = None;
        }
        let s = synth::generate();
        let song = Song {
            sample_rate: synth::SAMPLE_RATE,
            frames: s.truth.frames,
            stems: s
                .stems
                .into_iter()
                .map(|(name, st)| Stem {
                    name,
                    left: st.left.into(),
                    right: st.right.into(),
                })
                .collect(),
        };
        self.open_memory_song(song);
    }

    /// Adds an extra stem lane (for example "classic drums") and reloads.
    /// It replaces a lane of the same name.  The new lane plays from memory
    /// (D8 of M6), and each other lane keeps its audio and its file.
    pub fn add_stem(&mut self, name: &str, left: Vec<f32>, right: Vec<f32>) {
        let audio = LaneAudio::Memory {
            left: left.into(),
            right: right.into(),
        };
        self.add_lane(name, audio);
    }

    /// [`Self::add_stem`] of any lane audio, such as a stored lane of
    /// classic drums (D8 of the M7 plan).  A stored lane is refused, with
    /// a status line, if the stored lanes would no longer fit the
    /// read-ahead buffer (D2).
    pub fn add_lane(&mut self, name: &str, audio: LaneAudio) {
        if let Some(song) = &self.song {
            let mut lanes: Vec<ShownLane> = song
                .lanes
                .iter()
                .filter(|lane| lane.name != name)
                .cloned()
                .collect();
            if let Some(file) = audio.file() {
                let formats: Vec<_> = lanes
                    .iter()
                    .filter_map(|lane| lane.audio.file().map(|f| f.format))
                    .chain([file.format])
                    .collect();
                if !store_meta::fits_read_ahead(&formats) {
                    self.status = format!(
                        "{name} not added: the song's stored lanes would not fit the read-ahead buffer"
                    );
                    return;
                }
            }
            lanes.push(ShownLane {
                name: name.into(),
                audio,
            });
            let shown = Shown {
                sample_rate: song.sample_rate,
                frames: song.frames,
                lanes,
                mix_overview: song.mix_overview.clone(),
            };
            self.load_song(shown);
        }
    }

    /// Starts HPSS on the mix.  Natively it runs on a thread and the lane
    /// appears when it finishes; on the web it blocks the page while it runs.
    pub fn start_classic_drums(&mut self) {
        let Some(shown) = self.song.clone() else {
            return;
        };
        // D8 of the M7 plan: a separation and classic drums exclude each
        // other, so the worker never reads stems that a separation moves.
        if self.classic_running() || self.separation_running() {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        let limit = self.memory_limit;
        #[cfg(not(target_arch = "wasm32"))]
        let limit = MEMORY_PATH_MAX_FRAMES;
        match classic_route(&shown, limit) {
            ClassicRoute::Memory => {}
            #[cfg(target_arch = "wasm32")]
            ClassicRoute::Worker(inputs) => return self.classic_in_the_worker(&shown, &inputs),
            #[cfg(target_arch = "wasm32")]
            ClassicRoute::ReadFirst => return self.classic_read_first(&shown),
            ClassicRoute::TooLong => {
                self.status = format!(
                    "Classic drums not made: part of this song is in browser storage and part in memory, and it is longer than {}",
                    clock(limit)
                );
                return;
            }
            #[cfg(not(target_arch = "wasm32"))]
            ClassicRoute::Worker(_) | ClassicRoute::ReadFirst => {
                self.status = "Classic drums of stored lanes runs in the browser".into();
                return;
            }
        }
        let Some(song) = sources_in_memory(&shown) else {
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.classic_job.is_some() {
                return;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(spectrogram::classic_drums(&song));
            });
            self.classic_job = Some((self.song_serial, self.lanes_generation, rx));
            self.status = "Separating classic drums…".into();
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (l, r) = spectrogram::classic_drums(&song);
            self.add_stem("classic drums", l, r);
        }
    }

    /// True while a classic-drums job is running.
    pub fn classic_running(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.classic_job.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.classic_request.is_some()
        }
    }

    /// True while a separation starts or runs, or its stems are read.
    pub fn separation_running(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            self.separating.is_some() || self.reading_stems
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            false
        }
    }

    /// Classic drums of a song whose input lanes all have a session file:
    /// the decode worker writes them as a stored lane (D8 of the M7 plan).
    /// The session is protected from the request until the lane shows.
    #[cfg(target_arch = "wasm32")]
    fn classic_in_the_worker(&mut self, song: &Shown, inputs: &[store_meta::LaneFile]) {
        let Some(id) = self.session.clone() else {
            self.status = "Classic drums not made: the song is not kept in this browser".into();
            return;
        };
        let (serial, generation, frames) = (self.song_serial, self.lanes_generation, song.frames);
        self.classic_request = Some((serial, generation));
        self.status = "Separating classic drums…".into();
        let request = store::classic_request(&id, frames, inputs);
        let open = store::OpenGuard::new(&id);
        let guard = store::DecodeGuard::new(&id);
        let (pending, ctx, decoder) =
            (self.pending.clone(), self.ctx.clone(), self.decoder.clone());
        wasm_bindgen_futures::spawn_local(async move {
            use stem_protocol::decode::DecodeReply;
            let made = match decoder.request(&request, guard).await {
                decoder::Ended::Reply(DecodeReply::ClassicMade, guard) => {
                    let kept = store::keep_classic(&id, frames).await;
                    drop(guard);
                    kept.map(ClassicLane::Stored).map_err(|e| format!("{e:#}"))
                }
                decoder::Ended::Reply(DecodeReply::Unstored { text }, _) => Err(text),
                decoder::Ended::Reply(other, _) => {
                    Err(format!("the decode worker replied {}", other.kind()))
                }
                decoder::Ended::Stopped(why) => Err(why),
            };
            pending.borrow_mut().classic = Some(ClassicMade {
                serial,
                generation,
                made,
                _open: Some(open),
            });
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }

    /// Classic drums of a song whose input lanes are partly in browser
    /// storage: the page reads their mix first, a block at a time, into
    /// memory (the song fits the memory path), then runs the block HPSS on
    /// it (D8 of the M7 plan).
    #[cfg(target_arch = "wasm32")]
    fn classic_read_first(&mut self, song: &Shown) {
        let (serial, generation) = (self.song_serial, self.lanes_generation);
        self.classic_request = Some((serial, generation));
        self.status = "Reading the song from browser storage for classic drums…".into();
        let gains: Vec<f32> = song
            .lanes
            .iter()
            .map(|lane| {
                if DERIVED.contains(&lane.name.as_str()) {
                    0.0
                } else {
                    1.0
                }
            })
            .collect();
        let input = lanes::LanesMix::new(
            &job_audio(song, &self.lane_store),
            song.frames,
            song.sample_rate,
            &MixRecipe::new(gains),
        );
        let (pending, ctx, frames) = (self.pending.clone(), self.ctx.clone(), song.frames);
        wasm_bindgen_futures::spawn_local(async move {
            let mut job = export::ExportJob::new(Box::new(input), export::VecSink::new(frames));
            while !job.run_while(export::time_left(SAVE_MS)) {
                store::sleep(1).await;
            }
            let made = match job.failed() {
                Some(e) => Err(e),
                None => {
                    let mix = job.into_sink();
                    Ok(ClassicLane::Memory(spectrogram::percussive(
                        &mix.left, &mix.right,
                    )))
                }
            };
            pending.borrow_mut().classic = Some(ClassicMade {
                serial,
                generation,
                made,
                _open: None,
            });
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }

    /// Classic drums arrived: adds the lane, if its song is still shown and
    /// the lanes did not change; else discards it (D1 and D8 of the M7
    /// plan).
    #[cfg(target_arch = "wasm32")]
    fn classic_arrived(&mut self, made: ClassicMade) {
        self.classic_request = None;
        if made.serial != self.song_serial {
            self.status =
                "Classic drums finished for a song that is no longer open; discarded".into();
            return;
        }
        if made.generation != self.lanes_generation {
            self.status =
                "Classic drums finished for lanes that changed meanwhile; discarded".into();
            return;
        }
        let audio = match made.made {
            Ok(ClassicLane::Stored((file, overview))) => {
                let Some(store) = self.lane_store.clone() else {
                    return;
                };
                LaneAudio::Stored {
                    file,
                    overview: std::rc::Rc::new(overview),
                    store,
                }
            }
            Ok(ClassicLane::Memory((left, right))) => LaneAudio::Memory {
                left: left.into(),
                right: right.into(),
            },
            Err(e) => {
                log::warn!("classic drums failed: {e}");
                self.status = format!("Classic drums failed: {e}");
                return;
            }
        };
        // Keep the transport where it was.
        let head = self.params.as_ref().map(|p| p.playhead());
        self.add_lane("classic drums", audio);
        if let (Some(h), Some(p)) = (head, &self.params) {
            p.request_seek(h);
        }
    }

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((serial, generation, rx)) = &self.classic_job {
            match rx.try_recv() {
                Ok(_) if *serial != self.song_serial => {
                    self.classic_job = None;
                    self.status =
                        "Classic drums finished for a song that is no longer open; discarded"
                            .into();
                }
                Ok(_) if *generation != self.lanes_generation => {
                    self.classic_job = None;
                    self.status =
                        "Classic drums finished for lanes that changed meanwhile; discarded".into();
                }
                Ok((l, r)) => {
                    self.classic_job = None;
                    // Keep the transport where it was.
                    let head = self.params.as_ref().map(|p| p.playhead());
                    self.add_stem("classic drums", l, r);
                    if let (Some(h), Some(p)) = (head, &self.params) {
                        p.request_seek(h);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint(),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.classic_job = None;
                    self.status = "Classic drums job failed".into();
                }
            }
        }
        let _ = ctx;
    }

    pub fn is_playing(&self) -> bool {
        self.params
            .as_ref()
            .is_some_and(|p| p.playing.load(Relaxed))
    }

    /// Every 2 s while playing, logs the audio callback cost (to the browser
    /// console on the web).  This is how the in-browser engine cost is
    /// measured.
    fn log_audio_stats(&mut self, dt: f32) {
        self.stats_timer -= dt;
        if self.stats_timer > 0.0 || !self.is_playing() {
            return;
        }
        self.stats_timer = 2.0;
        let Some(p) = &self.params else { return };
        let n = p.callbacks.load(Relaxed);
        if n == 0 {
            log::info!("audio stats: no callbacks yet");
            return;
        }
        let sr = self.song.as_ref().map_or(44_100, |s| s.sample_rate) as f64;
        let frames = p.last_callback_frames.load(Relaxed) as f64;
        let budget_ms = frames / sr * 1000.0;
        let total = p.total_callback_ns.load(Relaxed);
        let (n0, t0) = self.stats_prev;
        self.stats_prev = (n, total);
        if n <= n0 {
            return;
        }
        // Mean over the callbacks since the previous line, so silent
        // callbacks before Play do not dilute it.
        let mean_ms = (total - t0) as f64 / (n - n0) as f64 / 1e6;
        let max_ms = p.max_callback_ns.load(Relaxed) as f64 / 1e6;
        log::info!(
            "audio stats: {} callbacks of {frames} frames ({budget_ms:.2} ms budget), engine mean {mean_ms:.3} ms ({:.1}%), max so far {max_ms:.3} ms ({:.1}%), tempo {:.2}, playhead {}",
            n - n0,
            mean_ms / budget_ms * 100.0,
            max_ms / budget_ms * 100.0,
            p.tempo(),
            p.playhead()
        );
        #[cfg(target_arch = "wasm32")]
        {
            let lanes = self.song.as_ref().map_or(0, |s| s.lanes.len());
            let held = p.held_frames.load(Relaxed);
            log::info!("{}", playback::line(self.playback.as_ref(), lanes, held));
        }
    }

    /// Advances the silent output by the frame time so the playhead moves.
    fn pump_silent(&mut self, dt: f32) {
        let Some(Output::Silent(player)) = &mut self.output else {
            return;
        };
        let sr = player.sample_rate() as f32;
        let mut frames = ((dt * sr) as usize).min(sr as usize);
        let (l, r) = &mut self.silent_scratch;
        while frames > 0 {
            let n = frames.min(l.len());
            player.process(&mut l[..n], &mut r[..n]);
            frames -= n;
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        // Nothing new is opened or stored while a reset runs.
        #[cfg(target_arch = "wasm32")]
        if matches!(self.reset, ResetStage::Running | ResetStage::Failed(..)) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // A single dropped folder loads every stem in it.
            if dropped.len() == 1 && dropped[0].path().is_dir() {
                match Song::load_folder(dropped[0].path()) {
                    Ok(s) => self.open_memory_song(s),
                    Err(e) => self.status = format!("Could not load folder: {e:#}"),
                }
                return;
            }
            let mut files = Vec::new();
            for f in &dropped {
                match f.bytes() {
                    Ok(b) => files.push((f.path().to_string_lossy().into_owned(), b)),
                    Err(e) => self.status = format!("Could not read {}: {e}", f.path().display()),
                }
            }
            self.load_files(files);
        }
        #[cfg(target_arch = "wasm32")]
        {
            // A dropped model file goes to the separation worker.
            let is_model = |f: &std::sync::Arc<dyn egui::DroppedFile + Send + Sync>| {
                f.path().to_string_lossy().ends_with(".safetensors")
            };
            let (models, dropped): (Vec<_>, Vec<_>) = dropped.into_iter().partition(is_model);
            for m in models {
                if let (Some(sep), Some(file)) = (&mut self.separator, m.web_file()) {
                    self.status = if sep.send_model_file(file) {
                        format!("Loading model {}…", m.path().display())
                    } else {
                        "Wait for the model or the separation to finish, then drop the model again"
                            .into()
                    };
                }
            }
            if dropped.is_empty() {
                return;
            }
            let pending = self.pending.clone();
            let ctx = ctx.clone();
            // D12: before any read or save.
            log_page_memory("as the open starts");
            let mut chosen = Vec::with_capacity(dropped.len());
            for f in &dropped {
                let name = f.path().to_string_lossy().into_owned();
                match f.web_file() {
                    Some(file) => chosen.push((name, file.clone())),
                    None => {
                        self.status = format!(
                            "Could not load: could not read {name}: the browser gave no file"
                        );
                        return;
                    }
                }
            }
            self.status = opening_text(&chosen);
            let decoder = self.decoder.clone();
            wasm_bindgen_futures::spawn_local(open_chosen(chosen, decoder, pending, ctx));
        }
    }

    /// "Open song…": the browser's file picker, the way to load a song on
    /// a phone or tablet.  Several files load as stems of one song, as a
    /// multi-file drop does.
    #[cfg(target_arch = "wasm32")]
    fn pick_song(&mut self, ctx: &egui::Context) {
        let (pending, ctx) = (self.pending.clone(), ctx.clone());
        let decoder = self.decoder.clone();
        picker::pick_files(SONG_FILES, true, move |files| {
            if files.is_empty() {
                return;
            }
            // D12: before any read or save.
            log_page_memory("as the open starts");
            let chosen: Chosen = files.into_iter().map(|f| (f.name(), f)).collect();
            pending.borrow_mut().status = Some(opening_text(&chosen));
            ctx.request_repaint();
            wasm_bindgen_futures::spawn_local(async move {
                // The "Opening…" text may still be queued; the result,
                // applied before the queued status, must not be hidden by it.
                let opened = open_chosen_now(&chosen, &decoder).await;
                {
                    let mut inbox = pending.borrow_mut();
                    inbox.status = None;
                    inbox.song = Some(opened);
                }
                ctx.request_repaint();
                refresh_storage(&pending).await;
            });
        });
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            #[cfg(target_arch = "wasm32")]
            if ui
                .button("Open song…")
                .on_hover_text("Choose an MP3, WAV or FLAC file, or several stems of one song")
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.pick_song(&ctx);
            }
            if ui.button("Demo song").clicked() {
                self.load_demo();
            }
            #[cfg(target_arch = "wasm32")]
            self.separator_controls(ui);
            #[cfg(target_arch = "wasm32")]
            self.storage_menu(ui);
            #[cfg(target_arch = "wasm32")]
            about_menu(ui);
            #[cfg(target_arch = "wasm32")]
            self.streamtest_controls(ui);
            let Some(params) = self.params.clone() else {
                return;
            };
            let playing = params.playing.load(Relaxed);
            if ui.button(if playing { "Pause" } else { "Play" }).clicked() {
                params.playing.store(!playing, Relaxed);
            }
            if ui.button("Rewind").clicked() {
                params.request_seek(0);
            }
            ui.separator();
            let slider = egui::Slider::new(&mut self.tempo_pct, 50.0..=150.0)
                .step_by(5.0)
                .suffix("%")
                .text("Tempo");
            if ui.add(slider).changed() {
                params.set_tempo(self.tempo_pct / 100.0);
            }
            let mut chip = params.chipmunk.load(Relaxed);
            if ui.toggle_value(&mut chip, "Chipmunk").changed() {
                params.chipmunk.store(chip, Relaxed);
            }
            if params.loop_region().is_some() && ui.button("Clear loop").clicked() {
                params.clear_loop();
            }
            ui.toggle_value(&mut self.show_spectrograms, "Spectrograms");
            let has_classic = self
                .song
                .as_ref()
                .is_some_and(|s| s.stem_index("classic drums").is_some());
            if !has_classic
                && ui
                    .add_enabled(
                        !self.classic_running() && !self.separation_running(),
                        egui::Button::new("Classic drums"),
                    )
                    .clicked()
            {
                self.start_classic_drums();
            }
            ui.separator();
            if let Some(song) = &self.song {
                let t = params.playhead() as f64 / song.sample_rate as f64;
                ui.monospace(format!("{:>5.1} s / {:.1} s", t, song.duration_s()));
            }
        });
        if self.params.is_some() {
            ui.label(&self.status);
        }
    }

    /// "Load model…", "Separate 6 stems" and the progress bar.
    #[cfg(target_arch = "wasm32")]
    fn separator_controls(&mut self, ui: &mut egui::Ui) {
        let Some(sep) = &mut self.separator else {
            return;
        };
        if sep.stopped.is_some() {
            ui.colored_label(ui.visuals().error_fg_color, "Separation stopped");
            if ui
                .button("Reset…")
                .on_hover_text("For when reloading the page does not bring separation back")
                .clicked()
            {
                self.reset = ResetStage::Asking;
            }
            return;
        }
        if sep.running {
            let (done, total, secs) = sep.progress;
            let frac = if total > 0 {
                done as f32 / total as f32
            } else {
                0.0
            };
            ui.add(
                egui::ProgressBar::new(frac)
                    .desired_width(260.0)
                    .text(format!("Separating: {done}/{total} chunks, {secs:.0} s")),
            );
            return;
        }
        if !sep.model_ready {
            if let Some((done, total)) = sep.download {
                // Hugging Face may not say the size; the file is 349.5 MB.
                let total = if total > 0.0 { total } else { 349_521_144.0 };
                let text = if done < total {
                    format!(
                        "Downloading model: {:.0} / {:.0} MB",
                        done / 1e6,
                        total / 1e6
                    )
                } else {
                    "Checking and loading model…".to_string()
                };
                ui.add(
                    egui::ProgressBar::new((done / total) as f32)
                        .desired_width(260.0)
                        .text(text),
                );
                return;
            }
            if sep.model_loading {
                ui.add_enabled(false, egui::Button::new("Loading model…"));
                return;
            }
            if ui
                .button("Download model (350 MB)")
                .on_hover_text(
                    "BS-RoFormer SW (6 stems), downloaded once from Hugging Face (huggingface.co/lumabeat/bs-roformer-sw) \
                     and kept in this browser.  Its trainer is unknown and no licence was ever stated for the weights.",
                )
                .clicked()
            {
                sep.download_model();
            }
            if ui
                .button("Load model file…")
                .on_hover_text("Choose bs_roformer_sw.f16.safetensors (BS-RoFormer SW, 6 stems) from this computer")
                .clicked()
            {
                sep.pick_model_file();
            }
            return;
        }
        if sep.model_loading {
            ui.add_enabled(false, egui::Button::new("Loading model…"));
            return;
        }
        if self.song.is_none() {
            return;
        }
        if !separate_enabled(self.separating.is_some(), self.reading_stems)
            || self.shown_session_decoding()
            || self.classic_running()
        {
            let why = if self.classic_running() {
                "Making classic drums…"
            } else if self.reading_stems {
                "Reading the separated stems from browser storage…"
            } else if self.separating.is_none() {
                "Opening the song from browser storage…"
            } else {
                "Starting the separation…"
            };
            ui.add_enabled(false, egui::Button::new("Separate 6 stems"))
                .on_disabled_hover_text(why);
            return;
        }
        if ui
            .button("Separate 6 stems")
            .on_hover_text("BS-RoFormer SW on the GPU: bass, drums, other, vocals, guitar, piano")
            .clicked()
        {
            self.start_separation(ui.ctx());
        }
    }

    /// Separate was pressed: records the start (D10 of M3), then asks the
    /// browser for its free storage and the session for its usable mix
    /// file, which choose the path when they arrive
    /// ([`Self::separation_ready`]).
    #[cfg(target_arch = "wasm32")]
    fn start_separation(&mut self, ctx: &egui::Context) {
        if !separate_enabled(self.separating.is_some(), self.reading_stems)
            || self.shown_session_decoding()
            || self.classic_running()
        {
            return;
        }
        let Some(song) = &self.song else {
            return;
        };
        if song.sample_rate != SEPARATION_RATE {
            self.status = format!(
                "Separation needs 44.1 kHz audio; this song is {} Hz",
                song.sample_rate
            );
            return;
        }
        let Some(job) = self.separation_job() else {
            return;
        };
        let serial = job.serial;
        self.separating = Some(job);
        self.status = "Starting the separation…".into();
        let (pending, ctx) = (self.pending.clone(), ctx.clone());
        let session = self.session.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let (used, quota) = store::usage().await;
            let mix = match &session {
                Some(id) => store::stored_mix_frames(id).await,
                None => None,
            };
            pending.borrow_mut().separation_start = Some((serial, (quota - used).max(0.0), mix));
            ctx.request_repaint();
        });
    }

    /// The browser's free storage and the session's usable mix file
    /// (`mix_frames`) arrived for the separation started on the song
    /// `serial`: chooses the path (D2 of M3, D7 of M4) and posts the
    /// request, if that song is still shown.  On the storage path, the
    /// page first writes the mix file if the session has none of the song.
    #[cfg(target_arch = "wasm32")]
    fn separation_ready(
        &mut self,
        serial: u64,
        free: f64,
        mix_frames: Option<usize>,
        ctx: &egui::Context,
    ) {
        let starting = self
            .separating
            .as_ref()
            .is_some_and(|j| j.serial == serial && j.stage == JobStage::Starting);
        if !starting {
            return;
        }
        let Some(song) = self.song.clone().filter(|_| serial == self.song_serial) else {
            self.separating = None;
            self.status =
                "Separation not started: another song was opened meanwhile.  Press Separate again."
                    .into();
            return;
        };
        let has_session = self.session.is_some();
        // D10 of the M7 plan: an input that is the mix file is kept.
        let input_is_mix = self
            .session
            .as_deref()
            .is_some_and(|id| input_is_the_mix_file(&song, &store::mix_path(id)));
        let mix_kept = has_session && (mix_frames == Some(song.frames) || input_is_mix);
        let reason = if has_session {
            let mix = if mix_kept {
                String::new()
            } else {
                format!(" and the mix {:.0} MB", 8.0 * song.frames as f64 / 1e6)
            };
            format!(
                "browser storage has {:.0} MB free, and the stems need {:.0} MB{mix}",
                free / 1e6,
                storage_needed(song.frames) / 1e6
            )
        } else {
            "this song is not kept in browser storage".into()
        };
        let needed = storage_path_needs(song.frames, mix_kept);
        match separation_path(has_session, song.frames, free, needed, self.memory_limit) {
            SeparationPath::Storage if mix_kept => self.post_separation(JobStage::Storage, None),
            SeparationPath::Storage => {
                let (input, _) = separation_input(&song, job_audio(&song, &self.lane_store));
                self.write_mix_first(input, ctx);
            }
            SeparationPath::Memory if has_session => self.separate_without_storage(&reason),
            SeparationPath::Memory => self.post_separation(JobStage::Memory, Some(reason)),
            SeparationPath::TooLong => {
                self.separating = None;
                self.status = refusal_text(song.frames, self.memory_limit, has_session, &reason);
                log::warn!("separator: {}", self.status);
            }
        }
    }

    /// Posts the starting separation of a song with a session on the
    /// memory path, because browser storage cannot take it for `reason`.
    /// Finding 1-4 of review round 1 of M3: the page then does not save
    /// the stems, as for D4 of M3.
    #[cfg(target_arch = "wasm32")]
    fn separate_without_storage(&mut self, reason: &str) {
        if let Some(job) = &mut self.separating {
            job.session = None;
            job.keeps_stems = false;
        }
        self.post_separation(
            JobStage::Memory,
            Some(format!("{reason}, so the stems will not be kept")),
        );
    }

    /// Writes the song's mix file to its session (D3 of M4), then posts on
    /// the storage path ([`Self::mix_written`]).  Separate and the
    /// session's Forget stay disabled meanwhile.
    #[cfg(target_arch = "wasm32")]
    fn write_mix_first(&mut self, input: Box<dyn engine::mixdown::Pcm>, ctx: &egui::Context) {
        let (Some(id), Some(job)) = (self.session.clone(), &mut self.separating) else {
            self.separating = None;
            return;
        };
        job.stage = JobStage::WritingMix;
        let serial = job.serial;
        self.status = "Writing the song's mix to browser storage for the separation…".into();
        log::info!(
            "separator: memory: page wasm {:.0} MB before the mix is written",
            separator::page_wasm_mb()
        );
        let (pending, ctx) = (self.pending.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let written = store::write_mix(&id, input)
                .await
                .map_err(|e| format!("{e:#}"));
            pending.borrow_mut().mix_written = Some((serial, written));
            ctx.request_repaint();
        });
    }

    /// The mix write of the separation started on the song `serial` ended:
    /// posts on the storage path if that song is still shown.  If the
    /// write failed, the song goes as a song with no room in storage (D7
    /// of M4).
    #[cfg(target_arch = "wasm32")]
    fn mix_written(&mut self, serial: u64, written: Result<(), String>) {
        let writing = self
            .separating
            .as_ref()
            .is_some_and(|j| j.serial == serial && j.stage == JobStage::WritingMix);
        if !writing {
            return;
        }
        let Some(frames) = self
            .song
            .as_ref()
            .filter(|_| serial == self.song_serial)
            .map(|s| s.frames)
        else {
            self.separating = None;
            self.status =
                "Separation not started: another song was opened meanwhile.  Press Separate again."
                    .into();
            return;
        };
        let Err(e) = written else {
            self.post_separation(JobStage::Storage, None);
            return;
        };
        let reason = format!("browser storage could not keep the song's mix ({e})");
        log::warn!("separator: {reason}");
        if frames <= self.memory_limit {
            self.separate_without_storage(&reason);
        } else {
            self.separating = None;
            self.status = refusal_text(frames, self.memory_limit, true, &reason);
        }
    }

    /// Posts the starting separation on `stage`'s path, for the shown song:
    /// on the storage path, from the session's mix file (D4 of M4); on the
    /// memory path, from two arrays, with `reason` for the status line.
    #[cfg(target_arch = "wasm32")]
    fn post_separation(&mut self, stage: JobStage, reason: Option<String>) {
        let Some(song) = self.song.clone() else {
            self.separating = None;
            return;
        };
        let to_storage = stage == JobStage::Storage && self.session.is_some();
        if !to_storage {
            // D9 of the M7 plan: an input in browser storage is read first.
            let (input, from_storage) = separation_input(&song, job_audio(&song, &self.lane_store));
            if from_storage {
                self.read_memory_input(input, stage, reason);
                return;
            }
        }
        let Some(sep) = &mut self.separator else {
            self.separating = None;
            return;
        };
        let sent = match (stage, &self.session) {
            (JobStage::Storage, Some(id)) => {
                // No samples cross: the worker reads the mix file, so a
                // removal of it that waits is cancelled (D10 of M6).
                store::keep_mix(id);
                let mix = stem_protocol::Mix::Stored {
                    path: store::mix_path(id),
                    frames: song.frames,
                };
                sep.separate(mix, Some(&store::session_folder(id)))
            }
            _ => {
                // Borrowed, not cloned: the worker gets its own copy
                // anyway, and this thread's WebAssembly memory never
                // shrinks once grown.  The same samples become the
                // "original" lane (see `with_original`).  A mix made here
                // is freed before the start line below.
                let Some(song) = song.as_song() else {
                    self.separating = None;
                    self.status = "Separation not started: the song is not in memory".into();
                    return;
                };
                let mixed;
                let (l, r): (&[f32], &[f32]) = match original_lane(&song) {
                    Some(lane) => (&lane.left, &lane.right),
                    None => {
                        mixed = mix_of_sources(&song);
                        (&mixed.0, &mixed.1)
                    }
                };
                let mix = stem_protocol::Mix::Arrays {
                    left: js_sys::Float32Array::from(l),
                    right: js_sys::Float32Array::from(r),
                };
                sep.separate(mix, None)
            }
        };
        self.separation_posted(sent, stage, reason);
    }

    /// Reads the memory path's input from browser storage, a block at a
    /// time, into two JS arrays, then posts it (D9 of the M7 plan).  The
    /// song is at most `memory_limit` frames long, so the arrays are
    /// bounded, and they are outside WebAssembly memory.
    #[cfg(target_arch = "wasm32")]
    fn read_memory_input(
        &mut self,
        input: Box<dyn engine::mixdown::Pcm>,
        stage: JobStage,
        reason: Option<String>,
    ) {
        let Some(serial) = self.separating.as_ref().map(|j| j.serial) else {
            return;
        };
        self.status = "Reading the song from browser storage for the separation…".into();
        let (pending, ctx) = (self.pending.clone(), self.ctx.clone());
        let frames = input.frames();
        wasm_bindgen_futures::spawn_local(async move {
            let mut job = export::ExportJob::new(input, export::JsChannels::new(frames));
            while !job.run_while(export::time_left(SAVE_MS)) {
                store::sleep(1).await;
            }
            let read = match job.failed() {
                Some(e) => Err(e),
                None => Ok(job.into_sink()),
            };
            pending.borrow_mut().memory_input = Some(MemoryInput {
                serial,
                stage,
                reason,
                read,
            });
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }

    /// The memory path's input arrived: posts it, if the separation that
    /// asked for it still waits for it.
    #[cfg(target_arch = "wasm32")]
    fn memory_input_read(&mut self, input: MemoryInput) {
        let waiting = input.serial == self.song_serial
            && self
                .separating
                .as_ref()
                .is_some_and(|j| j.serial == input.serial);
        if !waiting {
            return;
        }
        let channels = match input.read {
            Ok(channels) => channels,
            Err(e) => {
                self.separating = None;
                self.status = format!(
                    "Separation not started: the song could not be read from browser storage ({e})"
                );
                return;
            }
        };
        let Some(sep) = &mut self.separator else {
            self.separating = None;
            return;
        };
        let mix = stem_protocol::Mix::Arrays {
            left: channels.left,
            right: channels.right,
        };
        let sent = sep.separate(mix, None);
        self.separation_posted(sent, input.stage, input.reason);
    }

    /// After the post of a separation on `stage`'s path: its stage, its log
    /// lines and the status line.
    #[cfg(target_arch = "wasm32")]
    fn separation_posted(&mut self, sent: bool, stage: JobStage, reason: Option<String>) {
        if !sent {
            self.separating = None;
            self.status = "Could not reach the separation worker".into();
            return;
        }
        if let Some(job) = &mut self.separating {
            job.stage = stage;
        }
        let path = if stage == JobStage::Storage {
            "storage"
        } else {
            "memory"
        };
        log::info!("separator: separating on the {path} path");
        log::info!(
            "separator: memory: page wasm {:.0} MB as the separation starts",
            separator::page_wasm_mb()
        );
        self.status = match reason {
            None => "Separating; the stems go to browser storage as they are made…".into(),
            Some(reason) => format!("Separating in memory: {reason}…"),
        };
    }

    /// Browser storage failed during the running separation: it refused
    /// the stems, or could not give the stored mix.  The page removes the
    /// session's `mix` line and mix file (D6 of M4).  The shown song
    /// separates again in memory if it fits, and is refused if not (D4 of
    /// M3).
    #[cfg(target_arch = "wasm32")]
    fn separation_unstored(&mut self, text: String) {
        log::warn!("separator: browser storage failed: {text}");
        let Some(job) = self.separating.take() else {
            return;
        };
        if let Some(id) = job.session.clone() {
            // D6 of M4: the next start writes the mix again.  The removal
            // counts as a save from here, so the next start waits for it.
            let removal = store::forget_mix(&id);
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(e) = removal.await {
                    log::warn!("separator: could not remove the mix of session {id}: {e:#}");
                }
            });
        }
        let destination = job.destination(self.song_serial, self.session.as_deref());
        let frames = self.song.as_ref().map_or(0, |s| s.frames);
        let refused = format!("Browser storage failed ({text})");
        match after_unstored(&destination, frames, self.memory_limit) {
            AfterUnstored::Memory => {
                // Not saved: storage just refused them (D4).
                self.separating = self.separation_job().map(|j| SeparationJob {
                    session: None,
                    keeps_stems: false,
                    ..j
                });
                self.post_separation(
                    JobStage::Memory,
                    Some(format!(
                        "browser storage failed ({text}), so the stems will not be kept"
                    )),
                );
            }
            // D10 of M6: the song plays from its mix, which stays until
            // another song opens.
            AfterUnstored::TooLong if job.session.as_deref().is_some_and(store::protected) => {
                self.status = format!(
                    "{refused}.  To try again, open another song, then open this one again and press Separate."
                );
            }
            AfterUnstored::TooLong => {
                self.status = format!(
                    "{refused}.  {}.  Free space in the Storage menu, then press Separate again.",
                    too_long_text(frames, self.memory_limit)
                );
            }
            AfterUnstored::OpenAgain => {
                let name = job.name.as_deref().unwrap_or("the separated song");
                self.status = format!(
                    "{refused}, so \"{name}\" was not separated.  Open it, then press Separate again."
                );
            }
        }
    }

    /// A storage-path separation ended: moves its files into place and
    /// writes `meta` (D5 of M3), then reads the stems for a song shown or
    /// being opened (D6).
    #[cfg(target_arch = "wasm32")]
    fn separation_stored(
        &mut self,
        names: Vec<String>,
        len: usize,
        secs: f64,
        ctx: &egui::Context,
    ) {
        let Some(job) = self.separating.take() else {
            log::warn!("separator: stems were stored with no separation recorded");
            return;
        };
        let Some(id) = job.session.clone() else {
            self.status = "Separated, but the stems were stored for no song".into();
            return;
        };
        let destination = job.destination(self.song_serial, self.session.as_deref());
        let being_opened = self.opening.as_ref().is_some_and(|(_, o)| *o == id);
        let read = destination == StemsGo::Show || being_opened;
        self.reading_stems = read;
        self.status = format!("Separated in {secs:.0} s; putting the stems in place…");
        let (pending, ctx) = (self.pending.clone(), ctx.clone());
        let decoder = self.decoder.clone();
        // D10 of M6: the read protects the session until the stems show.
        let guard = read.then(|| store::OpenGuard::new(&id));
        wasm_bindgen_futures::spawn_local(async move {
            let stored = store::keep_stored_stems(&id, &names).await;
            // D7 of the M7 plan: the overviews of the new stems, before the
            // stems show.
            let overviews = match &stored.kept {
                Ok(()) => store::stem_overviews(&id, len, &decoder).await,
                Err(_) => None,
            };
            let not_kept = stored
                .kept
                .err()
                .map(|e| format!("Stems not kept in browser storage: {e:#}"));
            let stems = if read {
                let mut stems = Vec::with_capacity(names.len());
                let mut files = Vec::with_capacity(names.len());
                let mut failed = None;
                // One stem at a time, each read whole and decoded.  A stem
                // read from its `.part` file plays from memory (D8 of M6).
                for (name, path) in names.iter().zip(&stored.paths) {
                    match store::read_stem(path, name, len).await {
                        Ok((stem, file)) => {
                            stems.push(stem);
                            files.push(file);
                        }
                        Err(e) => {
                            failed = Some(e);
                            break;
                        }
                    }
                }
                Some(match failed {
                    Some(e) => Err(e),
                    None => Ok((stems, files)),
                })
            } else {
                None
            };
            let mix = if read {
                store::mix_lane_now(&id, len).await
            } else {
                None
            };
            pending.borrow_mut().stored = Some(StoredSeparation {
                job,
                stems,
                mix,
                secs,
                count: names.len(),
                not_kept,
                overviews,
                _guard: guard,
            });
            ctx.request_repaint();
            refresh_storage(&pending).await;
        });
    }

    /// `?streamtest`: the button that starts the storage playback test, and
    /// its last line.  iOS starts audio only after a tap, so the test waits
    /// for one.
    #[cfg(target_arch = "wasm32")]
    fn streamtest_controls(&mut self, ui: &mut egui::Ui) {
        let Some(status) = &self.streamtest else {
            return;
        };
        let (running, last) = {
            let s = status.borrow();
            (s.running, s.last.clone())
        };
        if ui
            .add_enabled(!running, egui::Button::new("Run storage test"))
            .on_hover_text("Measures playback from browser storage; the results go to the console")
            .clicked()
            && let Some(settings) = streamtest::Settings::from_url()
        {
            // eframe runs a click's frame in the pointer event handler,
            // where Safari lets audio start.
            let audio = streamtest::audio();
            streamtest::start(settings, audio, status.clone(), ui.ctx().clone());
        }
        if let Some(last) = last {
            ui.label(format!("Storage test: {last}"));
        }
    }

    /// What the browser keeps for this site: the song session and the model.
    #[cfg(target_arch = "wasm32")]
    fn storage_menu(&mut self, ui: &mut egui::Ui) {
        let (used, quota) = self.usage;
        let mut open = None;
        let mut forget = None;
        ui.menu_button("Storage", |ui| {
            ui.label(format!(
                "This browser keeps {:.0} MB for this page ({:.0} MB allowed): the model, and these songs with their stems.",
                used / 1e6,
                quota / 1e6
            ));
            ui.separator();
            if self.sessions.is_empty() {
                ui.label("No songs kept yet: open or drop one.");
            }
            for (id, meta) in &self.sessions {
                ui.horizontal(|ui| {
                    let current = self.session.as_deref() == Some(id.as_str());
                    let stems = if meta.stems.is_empty() { "" } else { ", separated" };
                    let label = format!("{}{stems}", meta.name);
                    if current {
                        ui.strong(label);
                    } else {
                        ui.label(label);
                    }
                    if !current
                        && ui
                            .add_enabled(self.opening.is_none(), egui::Button::new("Open").small())
                            .clicked()
                    {
                        open = Some(id.clone());
                        ui.close();
                    }
                    let protected = store::protected(id);
                    let can_forget = forget_enabled(id, self.separating.as_ref(), protected);
                    let why_not = if protected {
                        "This song plays from browser storage; open another song first"
                    } else {
                        "A separation is writing this song's stems; forget it once the separation ends"
                    };
                    if ui
                        .add_enabled(can_forget, egui::Button::new("Forget").small())
                        .on_hover_text("Delete this song and its stems from the browser")
                        .on_disabled_hover_text(why_not)
                        .clicked()
                    {
                        forget = Some(id.clone());
                        ui.close();
                    }
                });
            }
            ui.separator();
            let model = self
                .separator
                .as_ref()
                .is_some_and(|s| (s.model_ready || s.model_stored) && !s.model_loading && !s.running);
            if ui.add_enabled(model, egui::Button::new("Remove the model from this browser")).clicked() {
                if let Some(sep) = &mut self.separator {
                    sep.forget_model();
                }
                ui.close();
            }
            if ui
                .button("Reset…")
                .on_hover_text("Fetch this app's files again, or delete everything it keeps, then reload")
                .clicked()
            {
                self.reset = ResetStage::Asking;
                ui.close();
            }
        });
        let pending = self.pending.clone();
        if let Some(id) = open {
            self.status = "Opening the stored song…".into();
            self.opens += 1;
            let token = self.opens;
            self.opening = Some((token, id.clone()));
            let decoder = self.decoder.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(r) = restore_stored(&id, &decoder).await else {
                    {
                        let mut inbox = pending.borrow_mut();
                        inbox.song = Some(Err(anyhow::anyhow!(
                            "that song is no longer in browser storage"
                        )));
                        inbox.open_done = Some(token);
                    }
                    refresh_storage(&pending).await;
                    return;
                };
                {
                    let mut inbox = pending.borrow_mut();
                    inbox.open_done = Some(token);
                    match r {
                        Ok((opened, meta, note)) => {
                            let text = format!("Opened \"{}\" from this browser.", meta.name);
                            inbox.status = Some(match note {
                                Some(note) => format!("{text}  It was {note}."),
                                None => text,
                            });
                            inbox.song = Some(Ok(opened));
                        }
                        Err(e) => {
                            inbox.song = Some(Err(e.context("could not open the stored song")));
                        }
                    }
                }
                refresh_storage(&pending).await;
            });
        } else if let Some(id) = forget {
            if self.session.as_deref() == Some(id.as_str()) {
                self.session = None;
            }
            if let Some(job) = &mut self.separating
                && job.session.as_deref() == Some(id.as_str())
            {
                job.session = None;
            }
            self.status = "Forgetting the song…".into();
            wasm_bindgen_futures::spawn_local(async move {
                let text = if store::forget(&id).await {
                    "Song forgotten; if it is open, it stays open until another is loaded."
                } else {
                    "The browser did not delete that song; try again, or use Reset"
                };
                pending.borrow_mut().status = Some(text.into());
                refresh_storage(&pending).await;
            });
        }
    }

    /// The Reset dialog, then its progress until the page reloads, or what
    /// could not be deleted.
    #[cfg(target_arch = "wasm32")]
    fn reset_dialog(&mut self, ctx: &egui::Context) {
        match &self.reset {
            ResetStage::Closed => {}
            ResetStage::Asking => {
                let reason = self.separator.as_ref().and_then(|s| s.stopped.clone());
                let stored_mb = self.usage.0 / 1e6;
                match reset::dialog(ctx, reason.as_deref(), self.sessions.len(), stored_mb) {
                    reset::DialogOutcome::Open => {}
                    reset::DialogOutcome::Cancelled => self.reset = ResetStage::Closed,
                    reset::DialogOutcome::Chosen(choice) => self.start_reset(ctx, choice),
                }
            }
            ResetStage::Running => {
                reset::progress(ctx, None);
            }
            ResetStage::Failed(choice, failure) => match reset::progress(ctx, Some(failure)) {
                reset::ProgressOutcome::Waiting => {}
                reset::ProgressOutcome::TryAgain => self.start_reset(ctx, *choice),
                reset::ProgressOutcome::Reload => reset::reload(),
            },
        }
    }

    /// Ends the worker at once, and with it any separation, so that Forget
    /// comes back if a reset fails.  Separate stays hidden until a reload,
    /// because the worker stopped (`Separator::stopped`).
    #[cfg(target_arch = "wasm32")]
    fn stop_separation(&mut self) {
        if let Some(sep) = &mut self.separator {
            sep.stop();
        }
        self.separating = None;
        self.reading_stems = false;
    }

    /// Tells the store which session the separation names, so D7 keeps that
    /// session's `mix.f32` (D8).  The page calls it in each frame.
    #[cfg(target_arch = "wasm32")]
    fn publish_separating(&self) {
        let session = self
            .separating
            .as_ref()
            .filter(|job| job.stage != JobStage::Memory)
            .and_then(|job| job.session.clone());
        store::set_separating(session);
    }

    /// Whether the decode worker decodes the shown song's session now:
    /// Separate waits for it (D8).
    #[cfg(target_arch = "wasm32")]
    fn shown_session_decoding(&self) -> bool {
        self.session.as_deref().is_some_and(store::decoding)
    }

    /// All of a reset before `reset::run`: the dialog shows its progress,
    /// playback stops, and both workers end, so `store::close` waits for no
    /// decode (D9).
    #[cfg(target_arch = "wasm32")]
    fn prepare_reset(&mut self) {
        self.reset = ResetStage::Running;
        self.output = None;
        self.stop_workers();
    }

    /// Ends both workers before a reset (D9): first the decode worker, which
    /// drops the guard of each decode, so `store::close` never waits for a
    /// decode, then the separation worker.
    #[cfg(target_arch = "wasm32")]
    fn stop_workers(&mut self) {
        self.decoder.stop();
        self.stop_separation();
    }

    /// Stops playback and the worker, then runs the reset; the page reloads
    /// when it succeeds.
    #[cfg(target_arch = "wasm32")]
    fn start_reset(&mut self, ctx: &egui::Context, choice: reset::Reset) {
        self.prepare_reset();
        let (pending, ctx) = (self.pending.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(failure) = reset::run(choice).await {
                log::warn!("reset: {failure}");
                pending.borrow_mut().reset_failed = Some((choice, failure));
                ctx.request_repaint();
            }
        });
    }

    /// Applies the results of browser work that arrived since the last
    /// frame.
    #[cfg(target_arch = "wasm32")]
    fn apply_inbox(&mut self, ctx: &egui::Context) {
        let inbox = std::mem::take(&mut *self.pending.borrow_mut());
        if let Some(r) = inbox.song {
            match r {
                Ok(opened) => {
                    let holds_these = self
                        .stems_for_opening
                        .as_ref()
                        .is_some_and(|(id, ..)| opened.session.as_deref() == Some(id.as_str()));
                    let Opened {
                        song,
                        mix,
                        session,
                        name,
                        note,
                        guard,
                    } = opened;
                    // Stems held for this open play from memory (D8).
                    let song = match self.stems_for_opening.take_if(|_| holds_these) {
                        Some((_, stems, _)) => {
                            let stem_files = vec![None; stems.len()];
                            with_original_files(&song, stems, stem_files, mix)
                        }
                        None => song,
                    };
                    self.session = session;
                    self.song_name = name;
                    self.open_song(song);
                    // The song plays from its session now (D10 of M6).
                    drop(guard);
                    if let Some(note) = note {
                        self.status = note;
                    }
                }
                Err(e) => self.status = format!("Could not load: {e:#}"),
            }
        }
        if let Some(text) = inbox.status {
            self.status = text;
        }
        if let Some(token) = inbox.open_done
            && self.opening.as_ref().is_some_and(|(t, _)| *t == token)
        {
            self.opening = None;
            // Held stems this open did not show (it failed) are kept in
            // storage all the same.
            if let Some((.., kept)) = self.stems_for_opening.take() {
                self.status = kept;
            }
        }
        if let Some(u) = inbox.usage {
            self.usage = u;
        }
        if let Some(s) = inbox.sessions {
            self.sessions = s;
        }
        if let Some((choice, failure)) = inbox.reset_failed {
            self.reset = ResetStage::Failed(choice, failure);
        }
        if let Some((serial, free, mix)) = inbox.separation_start {
            self.separation_ready(serial, free, mix, ctx);
        }
        if let Some((serial, written)) = inbox.mix_written {
            self.mix_written(serial, written);
        }
        if let Some(input) = inbox.memory_input {
            self.memory_input_read(input);
        }
        if let Some(made) = inbox.classic {
            self.classic_arrived(made);
        }
        if let Some(stored) = inbox.stored {
            self.reading_stems = false;
            self.stored_stems_arrived(stored);
        }
    }

    /// Applies messages from the separation worker.
    #[cfg(target_arch = "wasm32")]
    fn poll_separator(&mut self, ctx: &egui::Context) {
        use separator::Event;
        let Some(sep) = &mut self.separator else {
            return;
        };
        let mut finished = None;
        let mut stored = None;
        let mut unstored = None;
        for ev in sep.take_events() {
            match ev {
                Event::ModelLoading => {
                    sep.model_loading = true;
                    self.status = "Loading model…".into();
                }
                Event::Download { done, total } => {
                    sep.download = Some((done, total));
                }
                Event::Model {
                    ok,
                    loaded,
                    stored,
                    text,
                } => {
                    sep.model_ready = loaded;
                    sep.model_stored = stored;
                    sep.model_loading = false;
                    sep.download = None;
                    let pending = self.pending.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        refresh_storage(&pending).await;
                    });
                    log::info!("separator: {text}");
                    self.status = text;
                    if sep.bench_pending {
                        sep.bench();
                        self.status = "Benchmark running (results in the browser console)…".into();
                    } else if ok && sep.selftest_pending {
                        sep.selftest();
                        self.status = "Self-test running (results in the browser console)…".into();
                    }
                }
                Event::Log(text) => log::info!("separator: {text}"),
                Event::Mp3 { name, bytes } => {
                    store::download(&name, &bytes, "audio/mpeg");
                    let note = self.save_notes.take(&name);
                    self.status = format!("Saved {name}.{note}");
                }
                Event::Flac { name, file } => {
                    store::download_blob(&name, &file);
                    let note = self.save_notes.take(&name);
                    self.status = format!("Saved {name}.{note}");
                }
                Event::Bench(text) => {
                    log::info!("separator: {text}");
                    self.status = text;
                }
                Event::Selftest { ok, text } => {
                    if ok {
                        log::info!("separator: {text}");
                    } else {
                        log::error!("separator: {text}");
                    }
                    self.status = text;
                }
                Event::Progress { done, total, secs } => {
                    sep.progress = (done, total, secs);
                    log::info!("separator: chunk {done}/{total} after {secs:.1} s");
                }
                Event::Stem { name, left, right } => sep.stems.push((name, left, right)),
                Event::Done { secs } => {
                    sep.running = false;
                    log_separation_end();
                    finished = Some((std::mem::take(&mut sep.stems), secs));
                }
                Event::StemsStored {
                    names, len, secs, ..
                } => {
                    sep.running = false;
                    log_separation_end();
                    stored = Some((names, len, secs));
                }
                Event::Unstored(text) => {
                    sep.running = false;
                    unstored = Some(text);
                }
                Event::Error { of, text } => {
                    log::error!("separator: {of}: {text}");
                    let what = match of.as_str() {
                        stem_protocol::kind::SEPARATE => {
                            sep.running = false;
                            self.separating = None;
                            "Separation"
                        }
                        stem_protocol::kind::MP3 | stem_protocol::kind::FLAC => {
                            // The failed file's note never gets its reply.
                            self.save_notes.clear();
                            if of == stem_protocol::kind::MP3 {
                                "MP3 encoding"
                            } else {
                                "FLAC encoding"
                            }
                        }
                        stem_protocol::kind::SELFTEST => "Self-test",
                        stem_protocol::kind::BENCH => "Benchmark",
                        _ => "Worker request",
                    };
                    self.status = format!("{what} failed: {text}");
                }
                Event::Status(text) => self.status = text,
                Event::Stopped(reason) => {
                    sep.running = false;
                    self.separating = None;
                    sep.model_loading = false;
                    sep.download = None;
                    log::error!("separator: the worker stopped: {reason}");
                    self.status = format!(
                        "The separation worker stopped ({reason}).  Reload the page; if that does not help, press Reset…"
                    );
                    sep.stopped = Some(reason);
                }
            }
        }
        if let Some((stems, secs)) = finished {
            let stems = stems
                .into_iter()
                .map(|(name, left, right)| Stem {
                    name,
                    left: left.into(),
                    right: right.into(),
                })
                .collect();
            match self.separating.take() {
                Some(job) => self.apply_separated(job, stems, secs),
                None => log::warn!("separator: stems arrived with no separation recorded"),
            }
        }
        if let Some((names, len, secs)) = stored {
            self.separation_stored(names, len, secs, ctx);
        }
        if let Some(text) = unstored {
            self.separation_unstored(text);
        }
    }

    /// Makes the mix lane show the mix as heard.  A new mix starts a
    /// waveform job, which runs for a few milliseconds in each frame.  The
    /// mix lane's spectrogram is then made again (D6 of the M7 plan).
    fn follow_mix(&mut self, ctx: &egui::Context) {
        self.follow_mix_while(ctx, export::time_left(PEAKS_MS));
    }

    /// [`Self::follow_mix`], while `more` says there is time.
    fn follow_mix_while(&mut self, ctx: &egui::Context, more: impl FnMut() -> bool) {
        let (Some(song), Some(params), Some(follower)) =
            (&self.song, &self.params, &mut self.mix_follower)
        else {
            return;
        };
        let heard = MixRecipe::as_heard(params);
        let store = &self.lane_store;
        let mix_of = |recipe: &MixRecipe| -> Box<dyn engine::mixdown::Pcm> {
            Box::new(lanes::LanesMix::new(
                &job_audio(song, store),
                song.frames,
                song.sample_rate,
                recipe,
            ))
        };
        let Some(waveform) = follower.follow(&heard, mix_of, more) else {
            // A source that waits asks for a repaint when its read ends.
            if follower.progress().is_some() && !follower.is_waiting() {
                ctx.request_repaint();
            }
            return;
        };
        if let Some(mix) = self.lanes.first_mut().filter(|l| l.stem.is_none()) {
            (mix.columns, mix.scale) = (waveform.columns, waveform.scale);
            mix.spectrogram = None;
            self.spectrograms.forget(0);
        }
    }

    /// Makes the spectrogram of each lane that lacks one, while
    /// spectrograms are on and `more` says there is time (D5 of the M7
    /// plan).  The mix lane shows the mix that its waveform shows.
    fn step_spectrograms(&mut self, ctx: &egui::Context, more: impl FnMut() -> bool) {
        let (Some(song), Some(follower)) = (&self.song, &self.mix_follower) else {
            return;
        };
        if !self.show_spectrograms {
            self.spectrograms = spectrogram::Spectrograms::default();
            return;
        }
        let audio = job_audio(song, &self.lane_store);
        let source = |lane: &Lane| -> Option<Box<dyn engine::mixdown::Pcm>> {
            Some(match lane.stem {
                Some(i) => Box::new(lanes::LanePcm::new(
                    audio[i].clone(),
                    song.frames,
                    song.sample_rate,
                )),
                None => Box::new(lanes::LanesMix::new(
                    &audio,
                    song.frames,
                    song.sample_rate,
                    follower.shown()?,
                )),
            })
        };
        self.spectrograms.step(ctx, &mut self.lanes, source, more);
    }

    /// A lane's "Save" menu: of its stem, or of the mix as heard for the
    /// mix lane.  It gives the format chosen.
    #[cfg(target_arch = "wasm32")]
    fn save_menu(
        &self,
        ui: &mut egui::Ui,
        song: &Shown,
        params: &SharedParams,
        stem: Option<usize>,
    ) -> Option<export::Format> {
        let heard = MixRecipe::as_heard(params);
        let why_not = if self.saving.is_some() {
            Some("Wait for the save in progress")
        } else if stem.is_none() && heard.is_silent() {
            Some("Unmute a stem to save a mix")
        } else {
            None
        };
        let mut chosen = None;
        ui.add_enabled_ui(why_not.is_none(), |ui| {
            ui.menu_button("Save", |ui| {
                if stem.is_none() {
                    let names: Vec<&str> = heard
                        .audible()
                        .into_iter()
                        .map(|i| song.lanes[i].name.as_str())
                        .collect();
                    ui.set_max_width(260.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "Saves what you hear: {}, at their volumes, at 100% tempo.",
                                names.join(", ")
                            ))
                            .small(),
                        )
                        .wrap(),
                    );
                    ui.separator();
                }
                for format in export::Format::ALL {
                    let can = !format.needs_worker() || self.separator.is_some();
                    if ui
                        .add_enabled(can, egui::Button::new(format.label()))
                        .clicked()
                    {
                        chosen = Some(format);
                        ui.close();
                    }
                }
            });
        })
        .response
        .on_disabled_hover_text(why_not.unwrap_or_default());
        chosen
    }

    /// Starts saving a stem, or the mix as heard when `stem` is `None`.
    /// The save runs in [`Self::step_save`], a few ms in each frame.
    #[cfg(target_arch = "wasm32")]
    fn start_save(&mut self, stem: Option<usize>, format: export::Format) {
        use engine::mixdown::Pcm;
        let (Some(song), Some(params)) = (self.song.clone(), self.params.clone()) else {
            return;
        };
        let base = self.song_name.as_deref().unwrap_or("song");
        let audio = job_audio(&song, &self.lane_store);
        let (frames, rate) = (song.frames, song.sample_rate);
        let (source, name): (Box<dyn Pcm>, String) = match stem {
            Some(i) => (
                Box::new(lanes::LanePcm::new(audio[i].clone(), frames, rate)),
                format!("{base} - {}", song.lanes[i].name),
            ),
            None => {
                let recipe = MixRecipe::as_heard(&params);
                let name = export::mix_file_name(base, &song.names(), &recipe);
                (
                    Box::new(lanes::LanesMix::new(&audio, frames, rate, &recipe)),
                    name,
                )
            }
        };
        let sink = export::JsSink::new(format, source.frames(), song.sample_rate);
        self.saving = Some(Saving {
            job: export::ExportJob::new(source, sink),
            name: format!("{name}.{}", format.extension()),
            rate: song.sample_rate,
            mix: stem.is_none(),
        });
    }

    /// Steps the save in progress for a few ms, then hands the file to the
    /// browser, or the channels to the worker.
    #[cfg(target_arch = "wasm32")]
    fn step_save(&mut self, ctx: &egui::Context) {
        use export::JsSink;
        let Some(saving) = &mut self.saving else {
            return;
        };
        if !saving.job.run_while(export::time_left(SAVE_MS)) {
            let doing = if saving.mix { "Mixing" } else { "Preparing" };
            let pct = 100.0 * saving.job.fraction();
            self.status = format!("{doing} {}: {pct:.0} %", saving.name);
            // A save that waits for storage gets a repaint when its read
            // ends.
            if !saving.job.is_waiting() {
                ctx.request_repaint();
            }
            return;
        }
        let Some(Saving {
            job,
            name,
            rate,
            mix,
        }) = self.saving.take()
        else {
            return;
        };
        let note = (mix && job.clipped() > 0).then(|| {
            format!(
                "  {} samples of the mix are at full scale, so it may clip; lower the volumes to avoid it.",
                job.clipped()
            )
        });
        let sep = self.separator.as_ref();
        self.status = match (job.into_sink(), sep) {
            (JsSink::Wav(wav), _) => {
                store::download(&name, &wav.into_bytes(), "audio/wav");
                format!("Saved {name}.{}", note.unwrap_or_default())
            }
            (JsSink::Flac(c), Some(sep)) => {
                sep.encode_flac(&name, c.left, c.right, rate);
                self.save_notes.push(&name, note);
                format!("Encoding {name}…")
            }
            (JsSink::Mp3(c), Some(sep)) => {
                sep.encode_mp3(&name, c.left, c.right, rate);
                self.save_notes.push(&name, note);
                format!("Encoding {name}…")
            }
            (JsSink::Flac(_) | JsSink::Mp3(_), None) => {
                format!("Could not save {name}: the encoding worker is not running.")
            }
        };
    }

    fn lane_controls(&mut self, ui: &mut egui::Ui, lane: usize) {
        let params = self.params.clone().expect("params");
        let l = &self.lanes[lane];
        let lane_stem = l.stem;
        ui.set_width(200.0);
        #[cfg(target_arch = "wasm32")]
        let mut chosen = None;
        ui.horizontal(|ui| {
            ui.colored_label(l.color, egui::RichText::new(&l.name).strong());
            #[cfg(target_arch = "wasm32")]
            if let Some(song) = &self.song {
                chosen = self.save_menu(ui, song, &params, lane_stem);
            }
        });
        #[cfg(target_arch = "wasm32")]
        if let Some(format) = chosen {
            self.start_save(lane_stem, format);
        }
        let Some(stem) = lane_stem else {
            if let Some(song) = &self.song {
                ui.small(export::mix_label(
                    &song.names(),
                    &MixRecipe::as_heard(&params),
                ));
            }
            return;
        };
        ui.horizontal(|ui| {
            let mut m = params.is_muted(stem);
            let mut changed = false;
            if ui.toggle_value(&mut m, "Mute").changed() {
                params.set_muted(stem, m);
                changed = true;
            }
            let mut s = params.is_soloed(stem);
            if ui.toggle_value(&mut s, "Solo").changed() {
                params.set_soloed(stem, s);
                changed = true;
            }
            let mut v = params.stems[stem].volume.get();
            if ui
                .add(egui::Slider::new(&mut v, 0.0..=1.5).show_value(false))
                .changed()
            {
                params.set_volume(stem, v);
                changed = true;
            }
            // The next frame redraws the mix lane for the new mix.
            if changed {
                ui.ctx().request_repaint();
            }
        });
    }

    fn lanes_view(&mut self, ui: &mut egui::Ui) {
        let (Some(song), Some(params)) = (self.song.clone(), self.params.clone()) else {
            ui.centered_and_justified(|ui| {
                ui.heading(&self.status);
            });
            return;
        };
        let frames = song.frames.max(1) as f64;
        let ctrl_w = 210.0;
        let wave_w = (ui.available_width() - ctrl_w).max(100.0);
        let x_of = |rect: Rect, f: u64| rect.left() + (f as f64 / frames) as f32 * rect.width();
        let frame_of = |rect: Rect, x: f32| {
            ((((x - rect.left()) / rect.width()).clamp(0.0, 1.0)) as f64 * frames) as u64
        };

        // Loop bar: drag to select a region.
        ui.horizontal(|ui| {
            ui.add_space(ctrl_w);
            let (resp, painter) =
                ui.allocate_painter(Vec2::new(wave_w, LOOP_BAR_HEIGHT), Sense::click_and_drag());
            let rect = resp.rect;
            self.loop_bar_rect = Some(rect);
            painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
            painter.text(
                rect.left_center() + Vec2::new(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "drag here to loop",
                egui::FontId::proportional(11.0),
                ui.visuals().weak_text_color(),
            );
            if resp.drag_started() {
                // Anchor where the button went down, not where the drag
                // threshold was crossed.
                let origin = ui.input(|i| i.pointer.press_origin());
                if let Some(p) = origin.or(resp.interact_pointer_pos()) {
                    self.loop_drag_anchor = Some(frame_of(rect, p.x));
                }
            }
            if resp.dragged()
                && let (Some(a), Some(p)) = (self.loop_drag_anchor, resp.interact_pointer_pos())
            {
                let b = frame_of(rect, p.x);
                params.set_loop(a.min(b), a.max(b));
            }
            if resp.drag_stopped() {
                self.loop_drag_anchor = None;
            }
            if let Some((s, e)) = params.loop_region() {
                let r = Rect::from_x_y_ranges(x_of(rect, s)..=x_of(rect, e), rect.y_range());
                painter.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(255, 210, 0, 160));
            }
        });

        let any_solo = (0..song.lanes.len()).any(|i| params.is_soloed(i));
        let mut rects = Vec::with_capacity(self.lanes.len());
        let mut seek_to = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for lane in 0..self.lanes.len() {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| self.lane_controls(ui, lane));
                    let (resp, painter) =
                        ui.allocate_painter(Vec2::new(wave_w, LANE_HEIGHT), Sense::click());
                    let rect = resp.rect;
                    rects.push(rect);
                    if resp.clicked()
                        && let Some(p) = resp.interact_pointer_pos()
                    {
                        seek_to = Some(frame_of(rect, p.x));
                    }
                    let l = &self.lanes[lane];
                    let audible = match l.stem {
                        None => true,
                        Some(i) => {
                            params.effective_gain(i) > 0.0 && (!any_solo || params.is_soloed(i))
                        }
                    };
                    let color = if audible {
                        l.color
                    } else {
                        Color32::from_gray(90)
                    };
                    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
                    if let Some((s, e)) = params.loop_region() {
                        let r =
                            Rect::from_x_y_ranges(x_of(rect, s)..=x_of(rect, e), rect.y_range());
                        painter.rect_filled(
                            r,
                            0.0,
                            Color32::from_rgba_unmultiplied(255, 210, 0, 40),
                        );
                    }
                    if self.show_spectrograms {
                        if let Some(tex) = &l.spectrogram {
                            painter.image(
                                tex.id(),
                                rect,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                if audible {
                                    Color32::WHITE
                                } else {
                                    Color32::from_gray(110)
                                },
                            );
                        }
                    } else {
                        draw_wave(&painter, rect, l, color);
                    }
                });
            }
        });
        // One playhead across every lane.
        if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
            let x = x_of(*first, params.playhead());
            ui.painter().line_segment(
                [Pos2::new(x, first.top()), Pos2::new(x, last.bottom())],
                Stroke::new(2.0, Color32::WHITE),
            );
        }
        self.lane_rects = rects;
        if let Some(f) = seek_to {
            params.request_seek(f);
        }
    }
}

fn draw_wave(painter: &egui::Painter, rect: Rect, lane: &Lane, color: Color32) {
    let n = lane.columns.len();
    if n == 0 {
        return;
    }
    let mid = rect.center().y;
    let half = rect.height() * 0.45 * lane.scale;
    let px = rect.width().max(1.0) as usize;
    for x in 0..px {
        let a = x * n / px;
        let b = ((x + 1) * n / px).max(a + 1).min(n);
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for c in &lane.columns[a..b] {
            lo = lo.min(c.0);
            hi = hi.max(c.1);
        }
        let xf = rect.left() + x as f32 + 0.5;
        painter.line_segment(
            [
                Pos2::new(xf, mid - hi * half),
                Pos2::new(xf, mid - lo * half - 0.5),
            ],
            Stroke::new(1.0, color),
        );
    }
}

impl eframe::App for StemApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // On a click, eframe runs this frame inside the browser's pointer
        // event handler, where a suspended Web Audio context may be resumed
        // (see `LiveOutput::resume`).
        #[cfg(feature = "audio")]
        if let Some(Output::Live(live)) = &self.output
            && ctx.input(|i| i.pointer.any_released())
            && let Err(e) = live.resume()
        {
            log::warn!("{e:#}");
        }
        self.handle_drops(&ctx);
        self.poll_jobs(&ctx);
        #[cfg(target_arch = "wasm32")]
        self.poll_separator(&ctx);
        #[cfg(target_arch = "wasm32")]
        self.apply_inbox(&ctx);
        let dt = ctx.input(|i| i.stable_dt);
        self.pump_silent(dt);
        self.log_audio_stats(dt);
        self.follow_mix(&ctx);
        #[cfg(target_arch = "wasm32")]
        self.step_save(&ctx);
        self.step_spectrograms(&ctx, export::time_left(SPECTROGRAM_MS));

        egui::Panel::top("transport").show(ui, |ui| {
            ui.add_space(4.0);
            self.transport(ui);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ui, |ui| self.lanes_view(ui));
        #[cfg(target_arch = "wasm32")]
        self.reset_dialog(&ctx);
        if self.is_playing() {
            ctx.request_repaint();
        }
        // After every change of this frame, before the browser work runs.
        #[cfg(target_arch = "wasm32")]
        self.publish_separating();
    }
}

/// The name of a separation's song, for the status line.
fn song_name(job: &SeparationJob) -> String {
    job.name
        .clone()
        .unwrap_or_else(|| "the separated song".into())
}

/// The status line for stems kept with a song that is not shown.
#[cfg(target_arch = "wasm32")]
fn kept_text(job: &SeparationJob, n: usize, secs: f64) -> String {
    format!(
        "Separated \"{}\" into {n} stems in {secs:.0} s and kept them in this browser; open it from Storage to play them.",
        song_name(job)
    )
}

/// `frames` at 44.1 kHz as minutes and seconds, such as "1:40".
pub fn clock(frames: usize) -> String {
    let seconds = frames / 44_100;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Why the memory path refuses a song of `frames`.
pub fn too_long_text(frames: usize, max_memory_frames: usize) -> String {
    format!(
        "this song is {} long, and without browser storage this page separates songs of up to {}",
        clock(frames),
        clock(max_memory_frames)
    )
}

/// The status line when the page refuses a song of `frames` before the
/// start (D2 of M3), for the `reason` that it does not take the storage
/// path.  `has_session`: whether the song is kept in this browser.
pub fn refusal_text(
    frames: usize,
    max_memory_frames: usize,
    has_session: bool,
    reason: &str,
) -> String {
    if has_session {
        format!(
            "Not separated: {}; {reason}.  Free space in the Storage menu, then press Separate again.",
            too_long_text(frames, max_memory_frames)
        )
    } else {
        // Only a song opened again gets a session, so free space alone
        // does not help (finding 3-4 of review round 3).
        format!(
            "Not separated: this song is {} long, and it is not kept in this browser.  A song over {} separates only from browser storage: open the file again once storage has room.",
            clock(frames),
            clock(max_memory_frames)
        )
    }
}

/// The page's memory when a separation's last reply arrives, before it
/// reads any stems (D9 of M3).
#[cfg(target_arch = "wasm32")]
fn log_separation_end() {
    log::info!(
        "separator: memory: page wasm {:.0} MB as the separation ends",
        separator::page_wasm_mb()
    );
}

/// Web entry point, called from `main` on wasm32.
#[cfg(target_arch = "wasm32")]
pub fn start_web() {
    use wasm_bindgen::JsCast;
    console_error_panic_hook::set_once();
    let _ = eframe::WebLogger::init(log::LevelFilter::Info);
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document
            .get_element_by_id("stem_explorer")
            .expect("canvas #stem_explorer")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .unwrap();
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(StemApp::new(cc)))),
            )
            .await;
        if let Err(e) = result {
            log::error!("failed to start eframe: {e:?}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_mix_makes_the_mix_lane_spectrogram_again() {
        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        app.load_demo();
        app.show_spectrograms = true;
        let ctx = egui::Context::default();
        app.step_spectrograms(&ctx, || true);
        let ids = |app: &StemApp| -> Vec<Option<egui::TextureId>> {
            app.lanes
                .iter()
                .map(|l| l.spectrogram.as_ref().map(|t| t.id()))
                .collect()
        };
        let before = ids(&app);
        assert!(
            before.iter().all(Option::is_some),
            "every lane in one frame"
        );

        let lead = app.song.as_ref().unwrap().stem_index("lead").unwrap();
        app.params.as_ref().unwrap().set_muted(lead, true);
        app.follow_mix_while(&ctx, || true);
        assert!(app.lanes[0].spectrogram.is_none(), "the mix lane's goes");
        app.step_spectrograms(&ctx, || true);
        let after = ids(&app);
        assert!(after[0].is_some());
        assert_ne!(after[0], before[0], "the mix lane's is made again");
        assert_eq!(after[1..], before[1..], "a stem's stays");
    }

    /// D1 of the M7 plan: classic drums that end after new lanes of the
    /// same song (new stems) are discarded, as for another song.
    #[test]
    fn classic_drums_for_lanes_that_changed_are_discarded() {
        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        app.load_demo();
        app.start_classic_drums();
        assert!(app.classic_running());
        let job = app.separation_job().expect("a job");
        let frames = app.song.as_ref().expect("a song").frames;
        let stems = vec![Stem {
            name: "bass".into(),
            left: vec![0.1; frames].into(),
            right: vec![0.1; frames].into(),
        }];
        app.apply_separated(job, stems, 1.0);
        let ctx = egui::Context::default();
        let start = std::time::Instant::now();
        while app.classic_running() {
            assert!(start.elapsed().as_secs() < 120, "classic drums never ended");
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.poll_jobs(&ctx);
        }
        let song = app.song.as_ref().expect("a song");
        assert_eq!(song.stem_index("classic drums"), None, "no lane added");
        assert!(app.status.contains("changed"), "status: {}", app.status);
    }

    /// D2 of the M7 plan: a stored lane that the read-ahead buffer does
    /// not play plays silence; it never falls back to samples.  A lane in
    /// memory shares its samples.  Natively, a stored song opens and its
    /// stored lanes play silence.
    #[test]
    fn a_stored_lane_with_no_read_ahead_plays_silence() {
        let store = lane_store::MemoryStore::new();
        store.put("acme/0.f32", vec![0; 4 * 8]);
        let stored = LaneAudio::Stored {
            file: store_meta::LaneFile::f32("acme/0.f32".into(), 4),
            overview: std::rc::Rc::new(lanes::Overview {
                columns: vec![(-0.5, 0.5); 4],
                scale: 2.0,
            }),
            store: store as std::rc::Rc<dyn lane_store::LaneStore>,
        };
        let memory = LaneAudio::Memory {
            left: vec![0.5; 4].into(),
            right: vec![0.25; 4].into(),
        };
        let shown = Shown {
            sample_rate: 44_100,
            frames: 4,
            mix_overview: None,
            lanes: vec![
                ShownLane {
                    name: "bass".into(),
                    audio: stored,
                },
                ShownLane {
                    name: "drums".into(),
                    audio: memory,
                },
            ],
        };
        let sources = memory_or_silent_sources(&shown);
        let read = |k: usize| {
            let (mut l, mut r) = ([9.0f32; 4], [9.0f32; 4]);
            sources[k].read(0, &mut l, &mut r);
            (l, r)
        };
        assert_eq!(read(0), ([0.0; 4], [0.0; 4]), "the stored lane is silent");
        assert_eq!(read(1), ([0.5; 4], [0.25; 4]), "the lane in memory plays");

        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        app.open_song(shown);
        assert_eq!(app.lanes.len(), 3, "the mix lane and two lanes");
        assert_eq!(
            app.lane_files()[0].as_ref().map(|f| f.path.as_str()),
            Some("acme/0.f32")
        );
    }

    /// The demo song as stored lanes in a store in memory, each an f32
    /// file with its overview, as the decode worker writes them, and the
    /// mix overview if `mix` is true.
    fn stored_demo(mix: bool) -> (Song, Shown) {
        let song = {
            let mut app = StemApp {
                want_device: false,
                ..StemApp::default()
            };
            app.load_demo();
            app.song
                .as_ref()
                .expect("a song")
                .as_song()
                .expect("in memory")
        };
        let store = lane_store::MemoryStore::new();
        let lanes = song
            .stems
            .iter()
            .map(|s| ShownLane {
                name: s.name.clone(),
                audio: lanes::tests::stored_f32(
                    &store,
                    &format!("demo/{}.f32", s.name),
                    &s.left,
                    &s.right,
                ),
            })
            .collect();
        let (ml, mr) = mix_of_sources(&song);
        let (columns, scale) = lane_columns(&ml, &mr);
        let shown = Shown {
            sample_rate: song.sample_rate,
            frames: song.frames,
            lanes,
            mix_overview: mix.then(|| std::rc::Rc::new(lanes::Overview { columns, scale })),
        };
        (song, shown)
    }

    fn app_showing(song: Shown) -> StemApp {
        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        app.open_song(song);
        app
    }

    /// D4 of the M7 plan: a stored lane draws from its overview, and the
    /// mix lane from the song's mix overview.
    #[test]
    fn a_stored_lane_draws_from_its_overview() {
        let (song, shown) = stored_demo(true);
        let app = app_showing(shown);
        for (k, stem) in song.stems.iter().enumerate() {
            let lane = &app.lanes[k + 1];
            assert_eq!(lane.name, stem.name);
            assert_eq!(
                (lane.columns.clone(), lane.scale),
                lane_columns(&stem.left, &stem.right)
            );
        }
        let (ml, mr) = mix_of_sources(&song);
        let mix = &app.lanes[0];
        assert_eq!((mix.columns.clone(), mix.scale), lane_columns(&ml, &mr));
        assert!(
            app.mix_follower
                .as_ref()
                .is_some_and(|f| f.shown().is_some())
        );
    }

    /// D4: a stored song with no mix overview starts with an empty mix
    /// lane, and the follower makes it in one frame, natively.
    #[test]
    fn a_mix_lane_with_no_overview_is_made_by_the_follower() {
        let (song, shown) = stored_demo(false);
        let mut app = app_showing(shown);
        assert!(app.lanes[0].columns.is_empty(), "no mix to draw yet");
        app.follow_mix_while(&egui::Context::default(), export::time_left(PEAKS_MS));
        let (ml, mr) = mix_of_sources(&song);
        let mix = &app.lanes[0];
        assert_eq!((mix.columns.clone(), mix.scale), lane_columns(&ml, &mr));
    }

    /// D2 of the M7 plan: a stored lane of classic drums is refused, with
    /// a status line, when the stored lanes would no longer fit the
    /// read-ahead buffer.  It replaces a lane of its name, so a new run of
    /// classic drums fits where the old one did.
    #[test]
    fn classic_drums_that_would_not_fit_the_read_ahead_are_refused() {
        use engine::readahead::LaneFormat;
        let store = lane_store::MemoryStore::new();
        let lane = |path: &str, format: LaneFormat| LaneAudio::Stored {
            file: store_meta::LaneFile {
                path: path.into(),
                format,
                data_offset: 0,
                data_frames: 4,
            },
            overview: std::rc::Rc::new(lanes::Overview {
                columns: vec![(0.0, 0.0); 4],
                scale: 1.0,
            }),
            store: std::rc::Rc::clone(&store) as std::rc::Rc<dyn lane_store::LaneStore>,
        };
        // 20 f32 lanes and two 16-bit lanes: 5,376 KiB a slot, which fits.
        let mut lanes: Vec<ShownLane> = (0..20)
            .map(|k| ShownLane {
                name: format!("acme {k}"),
                audio: lane(&format!("s/decoded/{k}.f32"), LaneFormat::F32),
            })
            .collect();
        for name in ["vocals", "classic drums"] {
            lanes.push(ShownLane {
                name: name.into(),
                audio: lane(&format!("s/{name}.wav"), LaneFormat::Pcm16),
            });
        }
        let mut app = app_showing(Shown {
            sample_rate: 44_100,
            frames: 4,
            lanes,
            mix_overview: None,
        });
        // A new run replaces the old lane: still 5,376 KiB.
        app.add_lane("classic drums", lane("s/classic.wav", LaneFormat::Pcm16));
        assert_eq!(app.lanes.len(), 23, "status: {}", app.status);
        assert_eq!(
            app.lane_files()[21].as_ref().map(|f| f.path.as_str()),
            Some("s/classic.wav")
        );
        // A third 16-bit lane: 5,504 KiB, which does not fit.
        app.add_lane("bass", lane("s/bass.wav", LaneFormat::Pcm16));
        assert_eq!(app.lanes.len(), 23, "the lane is not added");
        assert!(app.status.contains("read-ahead"), "status: {}", app.status);
        // A lane in memory takes no slot.
        app.add_stem("bass", vec![0.0; 4], vec![0.0; 4]);
        assert_eq!(app.lanes.len(), 24);
    }

    /// The samples of `pcm`, read whole: each read is ready at once.
    fn read_whole(pcm: &dyn engine::mixdown::Pcm) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0; pcm.frames()], vec![0.0; pcm.frames()]);
        assert!(pcm.ready(0, pcm.frames()));
        pcm.read(0, &mut l, &mut r);
        (l, r)
    }

    /// D9 of the M7 plan: the memory path's input is the lane that holds
    /// the original, as it is, else the mix of the sources, as
    /// `mix_of_sources` makes it.  It reads storage only when a lane that
    /// it takes is stored.
    #[test]
    fn the_memory_path_input_is_the_original_or_the_mix_of_the_sources() {
        let loud = |name: &str, level: f32| Stem {
            name: name.into(),
            left: vec![level, -level, 0.5, 0.25].into(),
            right: vec![0.5, level, -level, 0.0].into(),
        };
        // One source, louder than full scale, and classic drums.
        let one = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![loud("acme", 1.5), loud("classic drums", 0.75)],
        };
        let shown = Shown::in_memory(one.clone());
        let (input, stored) = separation_input(&shown, shown.audio());
        assert!(!stored);
        assert_eq!(
            read_whole(input.as_ref()),
            (
                one.stems[0].left.as_slice().to_owned(),
                one.stems[0].right.as_slice().to_owned()
            )
        );
        // Several sources: their mix, clamped.
        let several = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![
                loud("bass", 0.75),
                loud("drums", 0.5),
                loud("classic drums", 0.9),
            ],
        };
        let shown = Shown::in_memory(several.clone());
        let (input, stored) = separation_input(&shown, shown.audio());
        assert!(!stored);
        assert_eq!(read_whole(input.as_ref()), mix_of_sources(&several));
        // A stored source reads storage; a stored derived lane does not.
        let store = lane_store::MemoryStore::new();
        let stored_lane = |path: &str| LaneAudio::Stored {
            file: store_meta::LaneFile::f32(path.into(), 4),
            overview: std::rc::Rc::new(lanes::Overview {
                columns: vec![(0.0, 0.0); 4],
                scale: 1.0,
            }),
            store: std::rc::Rc::clone(&store) as std::rc::Rc<dyn lane_store::LaneStore>,
        };
        let mut audio = shown.audio();
        audio[2] = stored_lane("s/classic.wav");
        assert!(
            !separation_input(&shown, audio.clone()).1,
            "classic drums is not in the mix"
        );
        audio[1] = stored_lane("s/decoded/1.f32");
        assert!(separation_input(&shown, audio).1, "a stored source");
    }

    /// D10 of the M7 plan: when the memory path's input is the session's
    /// mix file, the mix is kept, so `write_mix` never reads the file it
    /// writes.
    #[test]
    fn the_mix_is_kept_when_the_input_is_the_mix_file() {
        let song = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("acme", 0.25), stem("classic drums", 0.5)],
        };
        let mix = "sessions/1/mix.f32";
        let from_mix = Shown::with_files(song.clone(), vec![file(mix), None]);
        assert!(input_is_the_mix_file(&from_mix, mix));
        let from_source =
            Shown::with_files(song.clone(), vec![file("sessions/1/decoded/0.f32"), None]);
        assert!(!input_is_the_mix_file(&from_source, mix));
        assert!(!input_is_the_mix_file(&Shown::in_memory(song), mix));
        // Several sources: the input is their mix, never one file.
        let several = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("bass", 0.25), stem("drums", 0.5)],
        };
        let shown = Shown::with_files(several, vec![file(mix), file("sessions/1/decoded/1.f32")]);
        assert!(!input_is_the_mix_file(&shown, mix));
    }

    /// D8 of the M7 plan: classic drums goes to the decode worker when
    /// every input lane has a file, stays on the page when every input lane
    /// is in memory, and reads the stored lanes first, or refuses, when
    /// they are mixed.  Derived lanes are not input.
    #[test]
    fn the_classic_drums_route_follows_where_the_input_lanes_are() {
        let song = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![
                stem("bass", 0.25),
                stem("drums", 0.5),
                stem("original", 0.1),
            ],
        };
        let route = |files: Vec<Option<store_meta::LaneFile>>, limit: usize| {
            classic_route(&Shown::with_files(song.clone(), files), limit)
        };
        assert_eq!(route(vec![None, None, None], 4), ClassicRoute::Memory);
        assert_eq!(
            route(vec![None, None, file("s/mix.f32")], 4),
            ClassicRoute::Memory,
            "the original is not input"
        );
        let stored = vec![file("s/stems/bass.wav"), file("s/stems/drums.wav"), None];
        assert_eq!(
            route(stored, 4),
            ClassicRoute::Worker(vec![
                store_meta::LaneFile::f32("s/stems/bass.wav".into(), 4),
                store_meta::LaneFile::f32("s/stems/drums.wav".into(), 4),
            ])
        );
        let mixed = || vec![file("s/stems/bass.wav"), None, None];
        assert_eq!(route(mixed(), 4), ClassicRoute::ReadFirst);
        assert_eq!(
            route(mixed(), 3),
            ClassicRoute::TooLong,
            "over the memory path's limit"
        );
    }

    #[test]
    fn spectrograms_off_make_no_texture() {
        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        app.load_demo();
        app.step_spectrograms(&egui::Context::default(), || true);
        assert!(app.lanes.iter().all(|l| l.spectrogram.is_none()));
    }

    fn job(serial: u64, session: Option<&str>) -> SeparationJob {
        SeparationJob {
            serial,
            session: session.map(String::from),
            name: None,
            stage: JobStage::Starting,
            keeps_stems: true,
        }
    }

    fn staged(session: &str, stage: JobStage) -> SeparationJob {
        SeparationJob {
            stage,
            ..job(1, Some(session))
        }
    }

    #[test]
    fn the_separation_path_follows_storage_and_length() {
        let limit = MEMORY_PATH_MAX_FRAMES;
        assert_eq!(limit, 4_410_000, "100 seconds at 44.1 kHz");
        let frames = 30 * 44_100;
        let needed = storage_needed(frames);
        assert_eq!(needed, 6.0 * (44.0 + 4.0 * frames as f64) + 16e6);
        // A session with enough space.
        assert_eq!(
            separation_path(true, frames, needed, needed, limit),
            SeparationPath::Storage
        );
        // A long song with a session and space has no limit.
        let long = 10 * limit;
        assert_eq!(
            separation_path(
                true,
                long,
                storage_needed(long),
                storage_needed(long),
                limit
            ),
            SeparationPath::Storage
        );
        // A session without enough space.
        assert_eq!(
            separation_path(true, frames, needed - 1.0, needed, limit),
            SeparationPath::Memory
        );
        // No session.
        assert_eq!(
            separation_path(false, frames, f64::INFINITY, needed, limit),
            SeparationPath::Memory
        );
        // At the limit and one frame over it.
        assert_eq!(
            separation_path(false, limit, 0.0, needed, limit),
            SeparationPath::Memory
        );
        assert_eq!(
            separation_path(false, limit + 1, 0.0, needed, limit),
            SeparationPath::TooLong
        );
        let over = storage_needed(limit + 1);
        assert_eq!(
            separation_path(true, limit + 1, over - 1.0, over, limit),
            SeparationPath::TooLong
        );
    }

    /// D7 of M4: a song with no usable mix file needs room for one too.
    #[test]
    fn the_separation_path_counts_a_missing_mix() {
        let limit = MEMORY_PATH_MAX_FRAMES;
        let frames = 30 * 44_100;
        let stems = storage_needed(frames);
        assert_eq!(storage_path_needs(frames, true), stems);
        let with_mix = storage_path_needs(frames, false);
        assert_eq!(with_mix, stems + 8.0 * frames as f64);
        assert_eq!(
            separation_path(true, frames, with_mix, with_mix, limit),
            SeparationPath::Storage
        );
        assert_eq!(
            separation_path(true, frames, with_mix - 1.0, with_mix, limit),
            SeparationPath::Memory,
            "room for the stems, not for the mix"
        );
        let long = 10 * limit;
        let long_needs = storage_path_needs(long, false);
        assert_eq!(
            separation_path(true, long, storage_needed(long), long_needs, limit),
            SeparationPath::TooLong
        );
    }

    #[test]
    fn a_clock_gives_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(44_099), "0:00", "whole seconds");
        assert_eq!(clock(100 * 44_100), "1:40");
        assert_eq!(clock(3_725 * 44_100), "62:05");
    }

    #[test]
    fn a_song_too_long_gives_its_length_and_the_limit() {
        assert_eq!(
            too_long_text(250 * 44_100, MEMORY_PATH_MAX_FRAMES),
            "this song is 4:10 long, and without browser storage this page separates songs of up to 1:40"
        );
    }

    /// Finding 3-4 of review round 3: free space does not help a long song
    /// with no session, so its refusal does not offer it.
    #[test]
    fn a_refusal_says_what_can_make_the_song_separate() {
        let long = 250 * 44_100;
        let reason = "browser storage has 0 MB free, and the stems need 280 MB";
        assert_eq!(
            refusal_text(long, MEMORY_PATH_MAX_FRAMES, true, reason),
            "Not separated: this song is 4:10 long, and without browser storage this page separates songs of up to 1:40; browser storage has 0 MB free, and the stems need 280 MB.  Free space in the Storage menu, then press Separate again."
        );
        let unkept = refusal_text(
            long,
            MEMORY_PATH_MAX_FRAMES,
            false,
            "this song is not kept in browser storage",
        );
        assert_eq!(
            unkept,
            "Not separated: this song is 4:10 long, and it is not kept in this browser.  A song over 1:40 separates only from browser storage: open the file again once storage has room."
        );
        assert!(!unkept.contains("press Separate again"));
    }

    #[test]
    fn a_separation_names_its_song() {
        let named = SeparationJob {
            name: Some("lloyd - tune".into()),
            ..job(1, None)
        };
        assert_eq!(song_name(&named), "lloyd - tune");
        assert_eq!(song_name(&job(1, None)), "the separated song");
    }

    /// A short song shown in a new app, and a job for it that keeps its
    /// stems or not.
    fn separated(keeps_stems: bool) -> String {
        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        app.open_memory_song(Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("mix", 0.4)],
        });
        let job = SeparationJob {
            keeps_stems,
            ..app.separation_job().expect("a job")
        };
        app.apply_separated(job, vec![stem("bass", 0.1), stem("drums", 0.2)], 3.0);
        app.status
    }

    #[test]
    fn stems_that_are_not_kept_say_so() {
        let not_kept = separated(false);
        assert!(
            not_kept.starts_with("Separated into 2 stems in 3 s.")
                && not_kept.ends_with("The stems are not kept in this browser."),
            "{not_kept}"
        );
        let kept = separated(true);
        assert!(!kept.contains("not kept"), "{kept}");
    }

    #[test]
    fn separate_waits_for_a_start_and_for_stored_stems() {
        assert!(separate_enabled(false, false));
        assert!(!separate_enabled(true, false), "a separation is starting");
        assert!(
            !separate_enabled(false, true),
            "stored stems are being read"
        );
        assert!(!separate_enabled(true, true));
    }

    #[test]
    fn forget_is_disabled_for_the_session_being_separated() {
        for stage in [JobStage::Starting, JobStage::WritingMix, JobStage::Storage] {
            let running = staged("a", stage);
            assert!(!forget_enabled("a", Some(&running), false), "{stage:?}");
            assert!(
                forget_enabled("b", Some(&running), false),
                "{stage:?}: another session"
            );
        }
        let memory = staged("a", JobStage::Memory);
        assert!(
            forget_enabled("a", Some(&memory), false),
            "the memory path writes no file"
        );
        assert!(forget_enabled("a", None, false), "no separation");
    }

    /// D10 of M6: no "Forget" for a session that the shown song plays
    /// from, or that an open reads.
    #[test]
    fn forget_is_not_offered_for_a_protected_session() {
        assert!(!forget_enabled("a", None, true));
        let memory = staged("a", JobStage::Memory);
        assert!(!forget_enabled("a", Some(&memory), true));
        assert!(forget_enabled("a", None, false));
    }

    #[test]
    fn an_unstored_separation_goes_to_memory_only_for_the_shown_song() {
        let limit = MEMORY_PATH_MAX_FRAMES;
        assert_eq!(
            after_unstored(&StemsGo::Show, limit, limit),
            AfterUnstored::Memory
        );
        assert_eq!(
            after_unstored(&StemsGo::Show, limit + 1, limit),
            AfterUnstored::TooLong
        );
        for other in [StemsGo::Kept, StemsGo::Discarded] {
            assert_eq!(
                after_unstored(&other, 100, limit),
                AfterUnstored::OpenAgain,
                "{other:?}"
            );
        }
    }

    fn stem(name: &str, v: f32) -> Stem {
        Stem {
            name: name.into(),
            left: vec![v; 4].into(),
            right: vec![-v; 4].into(),
        }
    }

    #[test]
    fn separated_stems_and_the_original_are_shared_not_copied() {
        let source = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("original", 0.5)],
        };
        let stems = vec![stem("bass", 0.1), stem("drums", 0.2)];
        let song = with_original(&source, stems.clone());
        assert_eq!(song.stems.len(), 3);
        for (a, b) in stems.iter().zip(&song.stems) {
            assert!(
                a.left.shares(&b.left) && a.right.shares(&b.right),
                "{}",
                a.name
            );
        }
        assert!(
            song.stems[2].left.shares(&source.stems[0].left),
            "the original"
        );

        // An opened song file is one stem: it is the original, not copied.
        let one_stem = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("song", 0.5)],
        };
        let song = with_original(&one_stem, stems.clone());
        let original = &song.stems[2];
        assert_eq!(original.name, "original");
        assert!(
            original.left.shares(&one_stem.stems[0].left)
                && original.right.shares(&one_stem.stems[0].right),
            "a one-stem source's original"
        );
    }

    #[test]
    fn a_lone_stem_is_the_original_unclamped_beside_derived_lanes() {
        let source = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("song", 1.5), stem("classic drums", 0.25)],
        };
        let lane = original_lane(&source).unwrap();
        assert!(lane.left.shares(&source.stems[0].left));
        let song = with_original(&source, vec![stem("bass", 0.1)]);
        assert_eq!(song.stems[1].left, vec![1.5; 4]);
        assert_eq!(song.stems[1].right, vec![-1.5; 4]);
    }

    #[test]
    fn several_source_stems_are_mixed_into_the_original() {
        let source = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("bass", 0.25), stem("drums", 0.5)],
        };
        assert!(original_lane(&source).is_none());
        let song = with_original(&source, vec![stem("vocals", 0.1)]);
        let original = &song.stems[1];
        assert_eq!(original.name, "original");
        assert_eq!(original.left, vec![0.75; 4]);
        assert_eq!(original.right, vec![-0.75; 4]);
        assert!(!original.left.shares(&source.stems[0].left));
    }

    fn file(path: &str) -> Option<store_meta::LaneFile> {
        Some(store_meta::LaneFile::f32(path.into(), 4))
    }

    /// D8 of M6: classic drums made on the page play from memory, and
    /// replace a lane of their name that is not the last; the other lanes
    /// keep their files.
    #[test]
    fn classic_drums_play_from_memory_and_the_other_lanes_keep_their_files() {
        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        let song = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![
                stem("classic drums", 0.1),
                stem("bass", 0.2),
                stem("drums", 0.3),
            ],
        };
        let files = vec![file("s/a.f32"), file("s/b.f32"), file("s/c.f32")];
        app.open_song(Shown::with_files(song, files));
        app.add_stem("classic drums", vec![0.5; 4], vec![0.5; 4]);
        let song = app.song.as_ref().expect("a song");
        assert_eq!(song.names(), ["bass", "drums", "classic drums"]);
        assert_eq!(app.lane_files(), [file("s/b.f32"), file("s/c.f32"), None]);
        assert_eq!(
            **song.lanes[2].audio.samples().expect("samples").0,
            [0.5; 4]
        );
    }

    /// D8 of M6: each lane keeps the file it was read from, and "original"
    /// plays from the file of the samples it takes.
    #[test]
    fn the_original_lane_plays_from_the_file_of_its_samples() {
        // One source: the original is that lane, with its file.
        let one = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("acme", 0.25), stem("classic drums", 0.5)],
        };
        let stems = vec![stem("vocals", 0.1), stem("bass", 0.2)];
        let left_of_one = one.stems[0].left.clone();
        let one = Shown::with_files(one, vec![file("s/mix.f32"), None]);
        let song = with_original_files(
            &one,
            stems.clone(),
            vec![file("s/stems/vocals.wav"), None],
            file("s/other.f32"),
        );
        assert_eq!(song.names(), ["vocals", "bass", "original"]);
        assert_eq!(
            song.files(),
            [file("s/stems/vocals.wav"), None, file("s/mix.f32")]
        );
        let (original, _) = song.lanes[2].audio.samples().expect("samples");
        assert!(original.shares(&left_of_one));
        // Several sources: the original is the mix file, if there is one.
        let several = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("bass", 0.25), stem("drums", 0.5)],
        };
        let sources = vec![file("s/decoded/0.f32"), file("s/decoded/1.f32")];
        let several = Shown::with_files(several, sources);
        let files =
            with_original_files(&several, stems.clone(), vec![None, None], file("s/mix.f32"))
                .files();
        assert_eq!(files, [None, None, file("s/mix.f32")]);
        let files = with_original_files(&several, stems.clone(), vec![None, None], None).files();
        assert_eq!(files, [None, None, None], "no usable mix: from memory");
        // A separated song separated again: "original" keeps its file.
        let separated =
            with_original_files(&several, stems.clone(), vec![None, None], file("s/mix.f32"));
        let files = with_original_files(
            &separated,
            stems,
            vec![file("s/stems/vocals.wav"), file("s/stems/bass.wav")],
            None,
        )
        .files();
        assert_eq!(
            files,
            [
                file("s/stems/vocals.wav"),
                file("s/stems/bass.wav"),
                file("s/mix.f32")
            ]
        );
    }

    #[test]
    fn an_original_lane_is_the_original_even_beside_one_stem() {
        let source = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![stem("bass", 0.25), stem("original", 0.5)],
        };
        let lane = original_lane(&source).unwrap();
        assert!(lane.left.shares(&source.stems[1].left));
    }

    #[test]
    fn stems_show_on_the_song_still_open() {
        assert_eq!(job(3, Some("a")).destination(3, Some("a")), StemsGo::Show);
        assert_eq!(job(3, None).destination(3, None), StemsGo::Show);
    }

    #[test]
    fn stems_show_on_the_song_opened_again_from_storage() {
        assert_eq!(job(3, Some("a")).destination(5, Some("a")), StemsGo::Show);
    }

    #[test]
    fn stems_are_only_kept_when_another_song_is_open() {
        assert_eq!(job(3, Some("a")).destination(4, Some("b")), StemsGo::Kept);
        assert_eq!(job(3, Some("a")).destination(4, None), StemsGo::Kept);
    }

    #[test]
    fn stems_of_an_unkept_song_no_longer_open_are_discarded() {
        assert_eq!(job(3, None).destination(4, None), StemsGo::Discarded);
        assert_eq!(job(3, None).destination(4, Some("b")), StemsGo::Discarded);
    }
}

/// Browser tests of how the page starts a separation and what it does when
/// browser storage refuses the stems (D2, D4 and D10 of M3), with a
/// `Separator` whose posts are kept, not sent (`cargo run -p xtask --
/// web-test`).  The worker's replies are delivered by hand.
#[cfg(all(test, target_arch = "wasm32"))]
mod separation_tests {
    use super::*;
    use stem_protocol::{Reply, Request, ToWorker};
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen(inline_js = r#"
// Keeps each message posted, and its transfer list in `posts.transfers`.
export function spy_on_post(worker) {
  const posts = [];
  posts.transfers = [];
  worker.postMessage = (data, transfer) => { posts.push(data); posts.transfers.push(transfer || []); };
  return posts;
}
export function transfers_of(posts) { return posts.transfers; }
// Counts the OPFS writes until `writes_counted`.
let counted = null;
let real_counted = null;
export function count_writes() {
  const proto = FileSystemWritableFileStream.prototype;
  real_counted = proto.write;
  counted = 0;
  proto.write = function (data) { counted++; return real_counted.call(this, data); };
}
export function writes_counted() {
  FileSystemWritableFileStream.prototype.write = real_counted;
  return counted;
}
export function deliver(worker, data) {
  worker.dispatchEvent(new MessageEvent("message", { data }));
}
// On the prototype: Safari ignores a property set on `navigator.storage`.
let real_estimate = null;
export function stub_estimate(usage, quota) {
  const proto = StorageManager.prototype;
  real_estimate = Object.getOwnPropertyDescriptor(proto, "estimate");
  Object.defineProperty(proto, "estimate", { value: async () => ({ usage, quota }), configurable: true, writable: true });
}
export function unstub_estimate() { Object.defineProperty(StorageManager.prototype, "estimate", real_estimate); }
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function wav_blob(bytes) { return new Blob([bytes]); }
// Browser storage refuses every write until `allow_writes`.
let real_write = null;
export function refuse_writes() {
  const proto = FileSystemWritableFileStream.prototype;
  real_write = proto.write;
  proto.write = () => Promise.reject(new DOMException("refused by the test", "QuotaExceededError"));
}
export function allow_writes() { FileSystemWritableFileStream.prototype.write = real_write; }
"#)]
    extern "C" {
        fn spy_on_post(worker: &web_sys::Worker) -> js_sys::Array;
        fn transfers_of(posts: &js_sys::Array) -> js_sys::Array;
        fn count_writes();
        fn writes_counted() -> u32;
        fn deliver(worker: &web_sys::Worker, data: &JsValue);
        fn stub_estimate(usage: f64, quota: f64);
        fn unstub_estimate();
        async fn sleep(ms: u32);
        fn wav_blob(bytes: &[u8]) -> web_sys::Blob;
        fn refuse_writes();
        fn allow_writes();
    }

    /// Two seconds: short for the memory path's limit, long for a limit of
    /// one second.
    const FRAMES: usize = 2 * 44_100;
    const ONE_SECOND: usize = 44_100;

    fn tone(frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| 0.25 * (0.01 * i as f32).sin())
            .collect()
    }

    /// A stored song of [`FRAMES`] shown with its session, the model
    /// loaded, and the page's posts kept in the returned array.
    async fn app_with_a_stored_song() -> (StemApp, js_sys::Array, String) {
        let wav = engine::wav::encode_pcm16(&tone(FRAMES), &tone(FRAMES), 44_100);
        let id = store::save_sources(&[("acme.wav".into(), wav_blob(&wav))], 44_100)
            .await
            .expect("a session");
        let mut sep = separator::Separator::new(egui::Context::default()).expect("a worker");
        // The test page has no worker script, so its error would stop it.
        sep.worker().set_onerror(None);
        sep.model_ready = true;
        sep.model_loading = false;
        let posts = spy_on_post(sep.worker());
        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        app.separator = Some(sep);
        app.open_memory_song(Song {
            sample_rate: 44_100,
            frames: FRAMES,
            stems: vec![Stem {
                name: "acme".into(),
                left: tone(FRAMES).into(),
                right: tone(FRAMES).into(),
            }],
        });
        app.session = Some(id.clone());
        app.song_name = Some("acme".into());
        (app, posts, id)
    }

    /// Presses Separate and waits for the browser's estimate, and for the
    /// mix write if the page writes the mix (D3 of M4).
    async fn press_separate(app: &mut StemApp) {
        app.start_separation(&egui::Context::default());
        settle(app).await;
        if app.separating.as_ref().map(|j| j.stage) == Some(JobStage::WritingMix) {
            settle(app).await;
        }
    }

    /// Waits for the page's browser work, then applies it.
    async fn settle(app: &mut StemApp) {
        for _ in 0..50 {
            sleep(20).await;
            let pending = app.pending.borrow();
            if pending.separation_start.is_some()
                || pending.stored.is_some()
                || pending.mix_written.is_some()
                || pending.memory_input.is_some()
            {
                break;
            }
        }
        app.apply_inbox(&egui::Context::default());
    }

    /// The bytes of the song of [`app_with_a_stored_song`] as a mix file:
    /// interleaved 32-bit little-endian floats.
    fn song_mix_file() -> Vec<u8> {
        tone(FRAMES)
            .iter()
            .flat_map(|v| [v.to_le_bytes(), v.to_le_bytes()])
            .flatten()
            .collect()
    }

    /// Session `id`'s `meta`.
    async fn meta_of(id: &str) -> store_meta::Meta {
        store::list()
            .await
            .into_iter()
            .find(|(got, _)| got == id)
            .map(|(_, meta)| meta)
            .unwrap_or_default()
    }

    /// Plants a mix file of `bytes` in session `id`, and a `mix` line of
    /// `frames` in its `meta` if given.
    async fn plant_mix(id: &str, bytes: &[u8], frames: Option<usize>) {
        store::test_write(&store::mix_path(id), bytes)
            .await
            .expect("planted");
        let mut meta = meta_of(id).await;
        meta.mix = frames.map(|f| (f, store_meta::MIX_FORMAT.into()));
        store::test_write(&format!("sessions/{id}/meta"), meta.to_text().as_bytes())
            .await
            .expect("planted");
    }

    /// The session of each `separate` request posted.
    fn sessions(posts: &js_sys::Array) -> Vec<Option<String>> {
        posts
            .iter()
            .map(
                |data| match ToWorker::from_js(&data).expect("a request").request {
                    Request::Separate { session, .. } => session,
                    other => panic!("not a separation: {other:?}"),
                },
            )
            .collect()
    }

    fn deliver_unstored(app: &mut StemApp, text: &str) {
        let (reply, _) = Reply::Unstored { text: text.into() }.to_js();
        let sep = app.separator.as_ref().expect("a separator");
        deliver(sep.worker(), &reply);
        app.poll_separator(&egui::Context::default());
    }

    /// The stems that the worker writes, in its order.
    const NAMES: [&str; 6] = ["bass", "drums", "other", "vocals", "guitar", "piano"];

    /// Stem `k`'s samples: a different tone for each stem.
    fn stem_wave(k: usize) -> Vec<f32> {
        let f = 0.002 * (k + 1) as f32;
        (0..FRAMES)
            .map(|i| 0.1 * (k + 1) as f32 * (f * i as f32).sin())
            .collect()
    }

    /// `x` as the 16-bit samples that a stem file holds.
    fn as_16_bits(x: &[f32]) -> Vec<f32> {
        x.iter()
            .map(|v| (v * 32768.0).round().clamp(-32768.0, 32767.0) / 32768.0)
            .collect()
    }

    /// The worker's finished `.part` file of each stem in `names`.
    async fn plant_parts(id: &str, names: &[&str]) {
        for (k, name) in NAMES.iter().enumerate().filter(|(_, n)| names.contains(n)) {
            let wave = stem_wave(k);
            let negated: Vec<f32> = wave.iter().map(|v| -v).collect();
            let path = format!("sessions/{id}/stems/{name}.wav.part");
            let wav = engine::wav::encode_pcm16(&wave, &negated, 44_100);
            store::test_write(&path, &wav).await.expect("planted");
        }
    }

    /// The worker's last `stored` reply and its `done`, then a frame.
    fn deliver_stored_and_done(app: &mut StemApp) {
        let sep = app.separator.as_ref().expect("a separator");
        let (stored, _) = Reply::Stored {
            frames: FRAMES,
            len: FRAMES,
            stems: NAMES.iter().map(|n| n.to_string()).collect(),
        }
        .to_js();
        deliver(sep.worker(), &stored);
        deliver(sep.worker(), &Reply::Done { secs: 2.0 }.to_js().0);
        app.poll_separator(&egui::Context::default());
    }

    /// The `stem` lines of session `id`'s `meta`.
    async fn stem_lines(id: &str) -> Vec<String> {
        store::list()
            .await
            .into_iter()
            .find(|(got, _)| got == id)
            .map(|(_, meta)| meta.stems)
            .unwrap_or_default()
    }

    /// D7 of the M7 plan: after a storage-path separation, the page asks
    /// the decode worker for the overviews of the new stems, and shows the
    /// stems only once they are in place, with the stems' mix overview.
    #[wasm_bindgen_test]
    async fn a_stored_separation_asks_for_stem_overviews_then_shows_them() {
        use stem_protocol::decode::{DecodeReply, WorkerRequest};
        let (mut app, _posts, id) = app_with_a_stored_song().await;
        let client = decoder::DecodeClient::new("./no-such-decode-worker.js");
        let worker = client.worker().expect("a worker");
        worker.set_onerror(None);
        let requests = spy_on_post(&worker);
        deliver(&worker, &DecodeReply::Alive.to_js().0);
        app.decoder = client;
        plant_parts(&id, &NAMES).await;
        press_separate(&mut app).await;
        deliver_stored_and_done(&mut app);
        for _ in 0..250 {
            if requests.length() > 0 {
                break;
            }
            sleep(20).await;
        }
        let req = match WorkerRequest::from_js(&requests.get(0)) {
            Ok(WorkerRequest::Overviews(req)) => req,
            other => panic!("not an overviews request: {other:?}"),
        };
        let lanes_before = app.lanes.len();
        let reading = app.reading_stems;
        // The worker's part files: the overview of each stem, then of their
        // mix, as `lane_columns` makes them.
        let sides: Vec<(Vec<f32>, Vec<f32>)> = (0..NAMES.len())
            .map(|k| {
                let wave = as_16_bits(&stem_wave(k));
                let negated: Vec<f32> = wave.iter().map(|v| -v).collect();
                (wave, negated)
            })
            .collect();
        let ones = vec![1.0; sides.len()];
        let lefts: Vec<&[f32]> = sides.iter().map(|(l, _)| l.as_slice()).collect();
        let rights: Vec<&[f32]> = sides.iter().map(|(_, r)| r.as_slice()).collect();
        let (ml, mr) = (dsp::mix(&lefts, &ones), dsp::mix(&rights, &ones));
        let file_of = |columns: &[(f32, f32)]| -> Vec<u8> {
            columns
                .iter()
                .flat_map(|(lo, hi)| [lo.to_le_bytes(), hi.to_le_bytes()])
                .flatten()
                .collect()
        };
        let mix_columns = lane_columns(&ml, &mr);
        let mut files: Vec<Vec<u8>> = sides
            .iter()
            .map(|(l, r)| file_of(&lane_columns(l, r).0))
            .collect();
        files.push(file_of(&mix_columns.0));
        let paths = req
            .lanes
            .iter()
            .map(|l| &l.overview)
            .chain(req.mix.as_ref());
        for (path, bytes) in paths.zip(&files) {
            store::test_write(&format!("{}/{path}", req.session), bytes)
                .await
                .expect("planted");
        }
        deliver(&worker, &DecodeReply::OverviewsMade.to_js().0);
        settle(&mut app).await;
        let shown = app.lanes.len();
        let mix_overview = app
            .song
            .as_ref()
            .and_then(|s| s.mix_overview.clone())
            .map(|o| (o.columns.clone(), o.scale));
        let meta = meta_of(&id).await;
        finish(app, &id).await;
        assert_eq!(req.lanes.len(), NAMES.len());
        assert!(reading, "the stems wait for their overviews");
        assert_eq!(
            lanes_before, 2,
            "the stems are not shown before their overviews"
        );
        assert_eq!(shown, 1 + NAMES.len() + 1, "the stems and the original");
        assert_eq!(mix_overview, Some(mix_columns), "the stems' mix overview");
        assert!(meta.stem_overviews_for(FRAMES), "{meta:?}");
    }

    /// A page showing a song whose lane plays from a session file, with a
    /// fake decode worker that said `decode-alive`: the page, its session,
    /// the worker and the posts to it.
    async fn app_with_a_stored_lane() -> (StemApp, String, web_sys::Worker, js_sys::Array) {
        let (mut app, _posts, id) = app_with_a_stored_song().await;
        let path = format!("sessions/{id}/decoded/9.f32");
        let bytes: Vec<u8> = tone(FRAMES)
            .iter()
            .flat_map(|v| [v.to_le_bytes(), v.to_le_bytes()])
            .flatten()
            .collect();
        store::test_write(&path, &bytes).await.expect("planted");
        let song = app
            .song
            .as_ref()
            .and_then(|s| s.as_song())
            .expect("in memory");
        app.load_song(Shown::with_files(
            song,
            vec![Some(store_meta::LaneFile::f32(path, FRAMES))],
        ));
        let client = decoder::DecodeClient::new("./no-such-decode-worker.js");
        let worker = client.worker().expect("a worker");
        worker.set_onerror(None);
        let requests = spy_on_post(&worker);
        deliver(
            &worker,
            &stem_protocol::decode::DecodeReply::Alive.to_js().0,
        );
        app.decoder = client;
        (app, id, worker, requests)
    }

    /// The first request posted to the decode worker, a `classic` one.
    async fn classic_request_of(requests: &js_sys::Array) -> stem_protocol::decode::ClassicRequest {
        for _ in 0..250 {
            if requests.length() > 0 {
                break;
            }
            sleep(20).await;
        }
        match stem_protocol::decode::WorkerRequest::from_js(&requests.get(0)) {
            Ok(stem_protocol::decode::WorkerRequest::Classic(req)) => req,
            other => panic!("not a classic request: {other:?}"),
        }
    }

    /// Plants the part files of `req` as the worker writes them, and gives
    /// the overview's columns.
    async fn plant_classic(req: &stem_protocol::decode::ClassicRequest) -> Vec<(f32, f32)> {
        let side: Vec<f32> = tone(FRAMES).iter().map(|v| 0.5 * v).collect();
        let wav = engine::wav::encode_pcm16(&side, &side, 44_100);
        let decoded = engine::decode::decode_bytes(wav.clone(), Some("wav")).expect("a WAV");
        let (columns, _) = lane_columns(&decoded.channels[0], &decoded.channels[1]);
        let overview: Vec<u8> = columns
            .iter()
            .flat_map(|(lo, hi)| [lo.to_le_bytes(), hi.to_le_bytes()])
            .flatten()
            .collect();
        store::test_write(&format!("{}/{}", req.session, req.wav), &wav)
            .await
            .expect("planted");
        store::test_write(&format!("{}/{}", req.session, req.overview), &overview)
            .await
            .expect("planted");
        columns
    }

    /// Waits for classic drums to arrive, then applies it.
    async fn classic_arrives(app: &mut StemApp) {
        for _ in 0..250 {
            sleep(20).await;
            if app.pending.borrow().classic.is_some() {
                break;
            }
        }
        app.apply_inbox(&egui::Context::default());
    }

    fn has_classic(app: &StemApp) -> bool {
        app.song
            .as_ref()
            .is_some_and(|s| s.stem_index("classic drums").is_some())
    }

    /// D8 of the M7 plan: classic drums of a song whose lanes are stored
    /// is a `classic` request of the decode worker.  The page moves its
    /// files into place, and adds a muted stored lane with its overview.
    #[wasm_bindgen_test]
    async fn classic_drums_of_a_stored_song_is_a_stored_lane() {
        let (mut app, id, worker, requests) = app_with_a_stored_lane().await;
        app.start_classic_drums();
        let running = app.classic_running();
        let req = classic_request_of(&requests).await;
        let columns = plant_classic(&req).await;
        deliver(
            &worker,
            &stem_protocol::decode::DecodeReply::ClassicMade.to_js().0,
        );
        classic_arrives(&mut app).await;
        let song = app.song.clone().expect("a song");
        let lane = song.lanes.last().expect("a lane").clone();
        let muted = app
            .params
            .as_ref()
            .is_some_and(|p| p.is_muted(song.lanes.len() - 1));
        let part = store::test_read(&format!("sessions/{id}/classic.wav.decode.part")).await;
        let meta = meta_of(&id).await;
        let still_running = app.classic_running();
        finish(app, &id).await;
        assert!(running, "the button waits");
        assert!(!still_running);
        assert_eq!(req.lanes.len(), 1);
        assert_eq!(req.lanes[0].path, "decoded/9.f32");
        assert_eq!(req.wav, "classic.wav.decode.part");
        assert_eq!(req.overview, "overviews/classic.f32.decode.part");
        assert_eq!(lane.name, "classic drums");
        match &lane.audio {
            LaneAudio::Stored { file, overview, .. } => {
                assert_eq!(file.path, format!("sessions/{id}/classic.wav"));
                assert_eq!(file.format, engine::readahead::LaneFormat::Pcm16);
                assert_eq!(overview.columns, columns);
            }
            _ => panic!("not a stored lane"),
        }
        assert!(muted, "a derived lane starts muted");
        assert_eq!(part, None, "the part file moved");
        assert!(meta.stems.is_empty(), "meta does not name classic drums");
    }

    #[wasm_bindgen_test]
    async fn classic_drums_for_a_song_no_longer_shown_is_discarded() {
        let (mut app, id, worker, requests) = app_with_a_stored_lane().await;
        app.start_classic_drums();
        let req = classic_request_of(&requests).await;
        plant_classic(&req).await;
        app.load_demo();
        deliver(
            &worker,
            &stem_protocol::decode::DecodeReply::ClassicMade.to_js().0,
        );
        classic_arrives(&mut app).await;
        let (added, status) = (has_classic(&app), app.status.clone());
        finish(app, &id).await;
        assert!(!added);
        assert!(status.contains("no longer open"), "{status}");
    }

    /// D1 of the M7 plan: new lanes of the same song (new stems) change
    /// the lanes generation, so classic drums of the old lanes is
    /// discarded.
    #[wasm_bindgen_test]
    async fn classic_drums_after_new_stems_is_discarded() {
        let (mut app, id, worker, requests) = app_with_a_stored_lane().await;
        app.start_classic_drums();
        let req = classic_request_of(&requests).await;
        plant_classic(&req).await;
        app.add_stem("vocals", vec![0.1; FRAMES], vec![0.1; FRAMES]);
        deliver(
            &worker,
            &stem_protocol::decode::DecodeReply::ClassicMade.to_js().0,
        );
        classic_arrives(&mut app).await;
        let (added, status) = (has_classic(&app), app.status.clone());
        finish(app, &id).await;
        assert!(!added);
        assert!(status.contains("changed"), "{status}");
    }

    /// D8 of the M7 plan: while classic drums runs, Separate does nothing,
    /// and while a separation runs, Classic drums does nothing.
    #[wasm_bindgen_test]
    async fn separate_and_classic_drums_exclude_each_other() {
        let (mut app, id, worker, requests) = app_with_a_stored_lane().await;
        app.start_classic_drums();
        let req = classic_request_of(&requests).await;
        app.start_separation(&egui::Context::default());
        let separating_during_classic = app.separating.is_some();
        plant_classic(&req).await;
        deliver(
            &worker,
            &stem_protocol::decode::DecodeReply::ClassicMade.to_js().0,
        );
        classic_arrives(&mut app).await;
        // A separation starts; Classic drums now does nothing.
        app.start_separation(&egui::Context::default());
        let separating = app.separating.is_some();
        let before = requests.length();
        app.start_classic_drums();
        let classic_during_separation = app.classic_running();
        let after = requests.length();
        finish(app, &id).await;
        assert!(
            !separating_during_classic,
            "Separate waits for classic drums"
        );
        assert!(separating, "a separation starts after it");
        assert!(
            !classic_during_separation,
            "classic drums waits for the separation"
        );
        assert_eq!(after, before, "no request during the separation");
    }

    #[wasm_bindgen_test]
    async fn a_failed_classic_request_adds_no_lane() {
        let (mut app, id, worker, requests) = app_with_a_stored_lane().await;
        app.start_classic_drums();
        classic_request_of(&requests).await;
        let unstored = stem_protocol::decode::DecodeReply::Unstored {
            text: "QuotaExceededError: the disk is full".into(),
        };
        deliver(&worker, &unstored.to_js().0);
        classic_arrives(&mut app).await;
        let (added, status, running) =
            (has_classic(&app), app.status.clone(), app.classic_running());
        finish(app, &id).await;
        assert!(!added);
        assert!(!running);
        assert!(status.contains("QuotaExceededError"), "{status}");
    }

    /// Findings 1-1 and 1-b3 of review round 1: at `done`, the page moves
    /// the stem files into place, writes `meta`, and shows the stems that
    /// it reads back, with Separate disabled until they are read.
    #[wasm_bindgen_test]
    async fn stored_stems_are_moved_read_and_shown() {
        let (mut app, _posts, id) = app_with_a_stored_song().await;
        plant_parts(&id, &NAMES).await;
        press_separate(&mut app).await;
        let stage = app.separating.as_ref().map(|j| j.stage);
        deliver_stored_and_done(&mut app);
        let reading = app.reading_stems;
        let enabled = separate_enabled(app.separating.is_some(), app.reading_stems);
        settle(&mut app).await;
        let read = app.reading_stems;
        let mut files = Vec::new();
        for name in NAMES {
            let base = format!("sessions/{id}/stems/{name}.wav");
            files.push((
                store::test_read(&base).await.is_some(),
                store::test_read(&format!("{base}.part")).await.is_some(),
            ));
        }
        let lines = stem_lines(&id).await;
        let song = app
            .song
            .as_ref()
            .expect("a song")
            .as_song()
            .expect("in memory");
        let status = app.status.clone();
        finish(app, &id).await;
        assert_eq!(stage, Some(JobStage::Storage));
        assert!(reading, "the stems are being read");
        assert!(!enabled, "Separate waits for the read");
        assert!(!read, "the read ended");
        assert_eq!(files, [(true, false); 6], "each .part moved onto its .wav");
        assert_eq!(lines, NAMES);
        let shown: Vec<&str> = song.stems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            shown,
            [
                "bass", "drums", "other", "vocals", "guitar", "piano", "original"
            ]
        );
        for (k, stem) in song.stems.iter().take(6).enumerate() {
            let want = as_16_bits(&stem_wave(k));
            let negated: Vec<f32> = want.iter().map(|v| -v).collect();
            assert!(*stem.left == want[..], "{}: the 16-bit samples", stem.name);
            assert!(
                *stem.right == negated[..],
                "{}: the 16-bit samples",
                stem.name
            );
        }
        assert!(status.starts_with("Separated into 6 stems"), "{status}");
    }

    /// Finding 1-1: the song separated is no longer shown, so the page
    /// reads no stems, and says where they are kept.
    #[wasm_bindgen_test]
    async fn stored_stems_of_a_song_no_longer_shown_are_kept_unread() {
        let (mut app, _posts, id) = app_with_a_stored_song().await;
        plant_parts(&id, &NAMES).await;
        press_separate(&mut app).await;
        app.open_memory_song(Song {
            sample_rate: 44_100,
            frames: FRAMES,
            stems: vec![Stem {
                name: "beachape".into(),
                left: tone(FRAMES).into(),
                right: tone(FRAMES).into(),
            }],
        });
        app.session = None;
        deliver_stored_and_done(&mut app);
        let reading = app.reading_stems;
        settle(&mut app).await;
        let lines = stem_lines(&id).await;
        let shown = app.song.as_ref().map_or(0, |s| s.lanes.len());
        let status = app.status.clone();
        finish(app, &id).await;
        assert!(!reading, "no read for a song not shown");
        assert_eq!(lines, NAMES, "kept");
        assert_eq!(shown, 1, "the shown song is as it was");
        assert!(status.contains("kept them in this browser"), "{status}");
    }

    /// Finding 1-1: a stem file that cannot move leaves `meta` without
    /// `stem` lines, and the status line says that the stems are not kept.
    #[wasm_bindgen_test]
    async fn a_stem_that_cannot_move_says_the_stems_are_not_kept() {
        let (mut app, _posts, id) = app_with_a_stored_song().await;
        plant_parts(&id, &NAMES[..5]).await;
        press_separate(&mut app).await;
        deliver_stored_and_done(&mut app);
        settle(&mut app).await;
        let lines = stem_lines(&id).await;
        let read = app.reading_stems;
        let status = app.status.clone();
        finish(app, &id).await;
        assert!(lines.is_empty(), "{lines:?}");
        assert!(!read);
        assert!(
            status.contains("Stems not kept in browser storage")
                && status.contains("piano.wav.part"),
            "{status}"
        );
    }

    /// Finding 1-14 of review round 1: a reset stops the worker, so no
    /// separation runs after it, and Forget comes back if the reset fails.
    /// Separate stays hidden until a reload, because the worker stopped
    /// (finding 2-10 of review round 2).
    #[wasm_bindgen_test]
    async fn stopping_the_worker_ends_the_separation() {
        let (mut app, _posts, id) = app_with_a_stored_song().await;
        press_separate(&mut app).await;
        let before = forget_enabled(&id, app.separating.as_ref(), false);
        app.reading_stems = true;
        app.stop_separation();
        let forget = forget_enabled(&id, app.separating.as_ref(), false);
        let reading = app.reading_stems;
        let stopped = app.separator.as_ref().and_then(|s| s.stopped.clone());
        finish(app, &id).await;
        assert!(!before, "Forget waits while the separation runs");
        assert!(forget, "Forget comes back");
        assert!(!reading, "no read goes on");
        assert_eq!(
            stopped.as_deref(),
            Some("stopped for a reset"),
            "Separate stays hidden until a reload"
        );
    }

    /// Finding 2-2 of review round 2: no new separation starts while the
    /// page reads stored stems back (D6) or while one runs, whatever
    /// calls the start.
    #[wasm_bindgen_test]
    async fn no_separation_starts_during_a_read_or_a_separation() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        app.reading_stems = true;
        press_separate(&mut app).await;
        let during_read = (posts.length(), app.separating.is_some());
        app.reading_stems = false;
        press_separate(&mut app).await;
        let first = app.separating.as_ref().map(|j| (j.serial, j.stage));
        press_separate(&mut app).await;
        let during_run = (
            posts.length(),
            app.separating.as_ref().map(|j| (j.serial, j.stage)),
        );
        finish(app, &id).await;
        assert_eq!(during_read, (0, false), "nothing starts during the read");
        assert_eq!(during_run, (1, first), "one request, and the job stays");
    }

    /// Finding 3-9 of review round 3: stems that are not kept, for a
    /// session that is being opened, keep the "not kept" text for a failed
    /// open.  Here `meta` cannot be written, so no file moves.
    #[wasm_bindgen_test]
    async fn stems_not_kept_for_a_song_being_opened_say_so() {
        let (mut app, _posts, id) = app_with_a_stored_song().await;
        plant_parts(&id, &NAMES).await;
        press_separate(&mut app).await;
        app.open_memory_song(Song {
            sample_rate: 44_100,
            frames: FRAMES,
            stems: vec![Stem {
                name: "beachape".into(),
                left: tone(FRAMES).into(),
                right: tone(FRAMES).into(),
            }],
        });
        app.session = None;
        app.opening = Some((1, id.clone()));
        refuse_writes();
        deliver_stored_and_done(&mut app);
        settle(&mut app).await;
        allow_writes();
        let held = app
            .stems_for_opening
            .as_ref()
            .map(|(held, stems, text)| (held.clone(), stems.len(), text.clone()));
        finish(app, &id).await;
        let (held, count, text) = held.expect("stems held for the open");
        assert_eq!((held, count), (id, 6));
        assert!(
            text.starts_with("Stems not kept in browser storage"),
            "{text}"
        );
    }

    /// Ends the test's worker and session.
    async fn finish(mut app: StemApp, id: &str) {
        if let Some(sep) = &mut app.separator {
            sep.stop();
        }
        store::forget(id).await;
    }

    /// The control for the tests below: without it, they would pass for a
    /// song with no session.
    /// D3 and D4 of M4: the page writes the mix file first, and the
    /// request names it with the song's frames, with no samples.
    #[wasm_bindgen_test]
    async fn a_song_with_a_session_and_space_takes_the_storage_path() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        app.start_separation(&egui::Context::default());
        settle(&mut app).await;
        let writing = app.separating.as_ref().map(|j| j.stage);
        let writing_forget = forget_enabled(&id, app.separating.as_ref(), false);
        let writing_separate = separate_enabled(app.separating.is_some(), app.reading_stems);
        let writing_posts = posts.length();
        settle(&mut app).await;
        let sent = sessions(&posts);
        let request = posts.get(0);
        let has_arrays = ["left", "right"].map(|k| {
            !js_sys::Reflect::get(&request, &k.into())
                .unwrap_or_default()
                .is_undefined()
        });
        let transfers: js_sys::Array = transfers_of(&posts).get(0).unchecked_into();
        let mix = match ToWorker::from_js(&request).map(|r| r.request) {
            Ok(Request::Separate {
                mix: stem_protocol::Mix::Stored { path, frames },
                ..
            }) => Some((path, frames)),
            _ => None,
        };
        let stage = app.separating.as_ref().map(|j| j.stage);
        let forget = forget_enabled(&id, app.separating.as_ref(), false);
        let status = app.status.clone();
        let file = store::test_read(&store::mix_path(&id)).await;
        let meta = meta_of(&id).await;
        finish(app, &id).await;
        assert_eq!(writing, Some(JobStage::WritingMix));
        assert!(!writing_forget, "no Forget while the page writes the mix");
        assert!(
            !writing_separate,
            "no Separate while the page writes the mix"
        );
        assert_eq!(writing_posts, 0, "the page posts after the mix write");
        assert_eq!(sent, [Some(format!("sessions/{id}"))]);
        assert_eq!(mix, Some((format!("sessions/{id}/mix.f32"), FRAMES)));
        assert_eq!(has_arrays, [false, false], "no left or right");
        assert_eq!(transfers.length(), 0, "nothing is transferred");
        assert!(file == Some(song_mix_file()), "the mix file holds the song");
        assert_eq!(meta.mix, Some((FRAMES, "f32le-1".into())));
        assert_eq!(stage, Some(JobStage::Storage));
        assert!(!forget, "D7: no Forget while the worker writes the session");
        assert!(status.contains("browser storage"), "{status}");
    }

    /// D7 of M4: a mix that storage refuses sends the song on the memory
    /// path, without its stems kept.
    #[wasm_bindgen_test]
    async fn a_failed_mix_write_takes_the_memory_path() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        refuse_writes();
        press_separate(&mut app).await;
        allow_writes();
        let sent = sessions(&posts);
        let transfers: js_sys::Array = transfers_of(&posts).get(0).unchecked_into();
        let job = app
            .separating
            .as_ref()
            .map(|j| (j.stage, j.keeps_stems, j.session.clone()));
        let status = app.status.clone();
        let meta = meta_of(&id).await;
        let file = store::test_read(&store::mix_path(&id)).await;
        let part = store::test_read(&format!("{}.part", store::mix_path(&id))).await;
        finish(app, &id).await;
        assert_eq!(sent, [None], "one request, on the memory path");
        assert_eq!(transfers.length(), 2, "MEM-2: the two arrays");
        assert_eq!(job, Some((JobStage::Memory, false, None)), "not saved");
        assert!(
            status
                .starts_with("Separating in memory: browser storage could not keep the song's mix")
                && status.contains("QuotaExceededError")
                && status.contains("will not be kept"),
            "{status}"
        );
        assert_eq!(meta.mix, None);
        assert_eq!((file, part), (None, None), "no mix file is left");
    }

    /// D7 of M4: storage has room for the stems, but not for the mix.  A
    /// song with no mix file takes the memory path.  A song with a usable
    /// mix file takes the storage path.
    #[wasm_bindgen_test]
    async fn the_free_space_must_hold_a_missing_mix() {
        let free = storage_needed(FRAMES) + 4.0 * FRAMES as f64;
        assert!(
            free < storage_path_needs(FRAMES, false),
            "no room for the mix"
        );
        stub_estimate(0.0, free);
        let (mut app, posts, id) = app_with_a_stored_song().await;
        press_separate(&mut app).await;
        let sent = sessions(&posts);
        let status = app.status.clone();
        finish(app, &id).await;
        let (mut kept, kept_posts, kept_id) = app_with_a_stored_song().await;
        plant_mix(&kept_id, &song_mix_file(), Some(FRAMES)).await;
        press_separate(&mut kept).await;
        let kept_sent = sessions(&kept_posts);
        let kept_stage = kept.separating.as_ref().map(|j| j.stage);
        finish(kept, &kept_id).await;
        unstub_estimate();
        assert_eq!(sent, [None], "no mix file: the memory path");
        assert!(
            status.starts_with("Separating in memory: browser storage has")
                && status.contains("and the mix 1 MB")
                && status.contains("will not be kept"),
            "{status}"
        );
        assert_eq!(
            kept_sent,
            [Some(format!("sessions/{kept_id}"))],
            "a usable mix file: the storage path"
        );
        assert_eq!(kept_stage, Some(JobStage::Storage));
    }

    /// Finding 3-1 of review round 3 of M4: a song opened while the page
    /// writes the mix is not separated, and the status line says why.
    #[wasm_bindgen_test]
    async fn a_song_opened_during_the_mix_write_is_not_separated() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        app.start_separation(&egui::Context::default());
        settle(&mut app).await;
        let writing = app.separating.as_ref().map(|j| j.stage);
        app.open_memory_song(Song {
            sample_rate: 44_100,
            frames: FRAMES,
            stems: vec![Stem {
                name: "beachape".into(),
                left: tone(FRAMES).into(),
                right: tone(FRAMES).into(),
            }],
        });
        settle(&mut app).await;
        let posted = posts.length();
        let running = app.separating.is_some();
        let status = app.status.clone();
        finish(app, &id).await;
        assert_eq!(writing, Some(JobStage::WritingMix));
        assert_eq!(posted, 0, "nothing posted");
        assert!(!running);
        assert!(status.contains("another song was opened"), "{status}");
    }

    /// Finding 3-2 of review round 3 of M4: a failed mix write of a song
    /// over the memory path's limit refuses the song (D7).
    #[wasm_bindgen_test]
    async fn a_failed_mix_write_refuses_a_long_song() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        app.memory_limit = ONE_SECOND;
        refuse_writes();
        press_separate(&mut app).await;
        allow_writes();
        let posted = posts.length();
        let running = app.separating.is_some();
        let status = app.status.clone();
        finish(app, &id).await;
        assert_eq!(posted, 0, "nothing posted");
        assert!(!running);
        assert!(
            status.starts_with("Not separated: this song is 0:02 long")
                && status.contains("browser storage could not keep the song's mix"),
            "{status}"
        );
    }

    /// D2 of M4: a usable mix file of the song is not written again.
    #[wasm_bindgen_test]
    async fn a_kept_mix_is_not_written_again() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        let kept = vec![7; 8 * FRAMES];
        plant_mix(&id, &kept, Some(FRAMES)).await;
        count_writes();
        press_separate(&mut app).await;
        let writes = writes_counted();
        let sent = sessions(&posts);
        let stage = app.separating.as_ref().map(|j| j.stage);
        let file = store::test_read(&store::mix_path(&id)).await;
        finish(app, &id).await;
        assert_eq!(writes, 0, "no write");
        assert!(file == Some(kept), "the kept mix file stays as it was");
        assert_eq!(sent, [Some(format!("sessions/{id}"))]);
        assert_eq!(stage, Some(JobStage::Storage));
    }

    /// D1 of M4: a mix file that `meta` does not name with the song's
    /// frames, or whose size is not that of its `mix` line, is written
    /// again.
    #[wasm_bindgen_test]
    async fn a_mix_without_its_meta_line_is_written_again() {
        for (case, bytes, line) in [
            ("no mix line", 8 * FRAMES, None),
            (
                "a mix line of other frames",
                8 * (FRAMES - 1),
                Some(FRAMES - 1),
            ),
            ("a file of another size", 8 * FRAMES - 8, Some(FRAMES)),
        ] {
            let (mut app, posts, id) = app_with_a_stored_song().await;
            plant_mix(&id, &vec![7; bytes], line).await;
            press_separate(&mut app).await;
            let sent = sessions(&posts);
            let file = store::test_read(&store::mix_path(&id)).await;
            let meta = meta_of(&id).await;
            finish(app, &id).await;
            assert!(file == Some(song_mix_file()), "{case}: written again");
            assert_eq!(meta.mix, Some((FRAMES, "f32le-1".into())), "{case}");
            assert_eq!(sent, [Some(format!("sessions/{id}"))], "{case}");
        }
    }

    /// D6 of M4: after `unstored`, the next start writes the mix again,
    /// and the status line says that browser storage failed.
    #[wasm_bindgen_test]
    async fn unstored_removes_the_mix_line() {
        let (mut app, _posts, id) = app_with_a_stored_song().await;
        press_separate(&mut app).await;
        let written = meta_of(&id).await.mix;
        deliver_unstored(&mut app, "NotReadableError: the mix file could not be read");
        let status = app.status.clone();
        let usable = store::stored_mix_frames(&id).await;
        let meta = meta_of(&id).await;
        let file = store::test_read(&store::mix_path(&id)).await;
        finish(app, &id).await;
        assert_eq!(written, Some((FRAMES, "f32le-1".into())), "the control");
        assert_eq!(usable, None);
        assert_eq!(meta.mix, None, "no mix line");
        assert_eq!(file, None, "no mix file");
        assert!(
            status.contains("browser storage failed (NotReadableError"),
            "{status}"
        );
    }

    #[wasm_bindgen_test]
    async fn unstored_separates_a_short_song_in_memory() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        press_separate(&mut app).await;
        posts.set_length(0);
        deliver_unstored(&mut app, "QuotaExceededError: the disk is full");
        // Any estimate that the page asked for would arrive meanwhile.
        settle(&mut app).await;
        let sent = sessions(&posts);
        let job = app
            .separating
            .as_ref()
            .map(|j| (j.stage, j.keeps_stems, j.session.clone()));
        let status = app.status.clone();
        finish(app, &id).await;
        assert_eq!(sent, [None], "one request, on the memory path");
        assert_eq!(job, Some((JobStage::Memory, false, None)), "not saved");
        assert!(status.contains("Separating in memory"), "{status}");
        assert!(
            status.contains("QuotaExceededError: the disk is full"),
            "{status}"
        );
    }

    /// D9 of the M7 plan: on the memory path, a song whose input is in
    /// browser storage posts the input read from its file.  The file holds
    /// other samples than the page's memory, so the post shows which were
    /// read.
    #[wasm_bindgen_test]
    async fn the_memory_path_reads_a_stored_input() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        let path = format!("sessions/{id}/decoded/9.f32");
        let file_side: Vec<f32> = tone(FRAMES).iter().map(|v| -0.5 * v).collect();
        let bytes: Vec<u8> = file_side
            .iter()
            .flat_map(|v| [v.to_le_bytes(), v.to_le_bytes()])
            .flatten()
            .collect();
        store::test_write(&path, &bytes).await.expect("planted");
        let song = app
            .song
            .as_ref()
            .and_then(|s| s.as_song())
            .expect("in memory");
        let file = store_meta::LaneFile::f32(path, FRAMES);
        app.load_song(Shown::with_files(song, vec![Some(file)]));
        press_separate(&mut app).await;
        posts.set_length(0);
        deliver_unstored(&mut app, "QuotaExceededError: the disk is full");
        settle(&mut app).await;
        let posted: Vec<(Vec<f32>, Vec<f32>)> = posts
            .iter()
            .map(
                |data| match ToWorker::from_js(&data).expect("a request").request {
                    Request::Separate {
                        mix: stem_protocol::Mix::Arrays { left, right },
                        session: None,
                    } => {
                        let (mut l, mut r) = (
                            vec![0.0; left.length() as usize],
                            vec![0.0; right.length() as usize],
                        );
                        left.copy_to(&mut l);
                        right.copy_to(&mut r);
                        (l, r)
                    }
                    other => panic!("not a separation in memory: {other:?}"),
                },
            )
            .collect();
        let status = app.status.clone();
        finish(app, &id).await;
        assert_eq!(posted.len(), 1, "one request, on the memory path");
        assert!(
            posted[0].0 == file_side && posted[0].1 == file_side,
            "the file's samples"
        );
        assert!(status.contains("Separating in memory"), "{status}");
    }

    #[wasm_bindgen_test]
    async fn unstored_refuses_a_long_song() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        app.memory_limit = ONE_SECOND;
        press_separate(&mut app).await;
        posts.set_length(0);
        deliver_unstored(&mut app, "QuotaExceededError: the disk is full");
        settle(&mut app).await;
        let posted = posts.length();
        let running = app.separating.is_some();
        let status = app.status.clone();
        finish(app, &id).await;
        assert_eq!(posted, 0, "nothing posted");
        assert!(!running);
        assert!(status.contains("QuotaExceededError"), "{status}");
        assert!(
            status.contains("0:02 long") && status.contains("up to 0:01"),
            "the length and the limit: {status}"
        );
    }

    #[wasm_bindgen_test]
    async fn a_full_storage_chooses_the_memory_path_before_the_start() {
        stub_estimate(1e9, 1e9);
        let (mut short, short_posts, short_id) = app_with_a_stored_song().await;
        press_separate(&mut short).await;
        let short_sent = sessions(&short_posts);
        let short_job = short
            .separating
            .as_ref()
            .map(|j| (j.stage, j.keeps_stems, j.session.clone()));
        let short_status = short.status.clone();
        finish(short, &short_id).await;
        let (mut long, long_posts, long_id) = app_with_a_stored_song().await;
        long.memory_limit = ONE_SECOND;
        press_separate(&mut long).await;
        let long_posted = long_posts.length();
        let long_status = long.status.clone();
        finish(long, &long_id).await;
        unstub_estimate();
        assert_eq!(short_sent, [None], "the memory path");
        assert_eq!(
            short_job,
            Some((JobStage::Memory, false, None)),
            "finding 1-4: no save into a full storage"
        );
        assert!(short_status.contains("will not be kept"), "{short_status}");
        assert!(
            short_status.contains("Separating in memory: browser storage has 0 MB free"),
            "{short_status}"
        );
        assert_eq!(long_posted, 0, "refused");
        assert!(
            long_status.starts_with("Not separated: this song is 0:02 long")
                && long_status.contains("up to 0:01"),
            "{long_status}"
        );
    }

    #[wasm_bindgen_test]
    async fn a_start_for_a_song_no_longer_shown_posts_nothing() {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        app.start_separation(&egui::Context::default());
        let starting = app.separating.as_ref().map(|j| j.stage);
        let forget = forget_enabled(&id, app.separating.as_ref(), false);
        app.open_memory_song(Song {
            sample_rate: 44_100,
            frames: FRAMES,
            stems: vec![Stem {
                name: "beachape".into(),
                left: tone(FRAMES).into(),
                right: tone(FRAMES).into(),
            }],
        });
        settle(&mut app).await;
        let posted = posts.length();
        let running = app.separating.is_some();
        let status = app.status.clone();
        finish(app, &id).await;
        assert_eq!(starting, Some(JobStage::Starting));
        assert!(!forget, "D7: no Forget while the separation starts");
        assert_eq!(posted, 0, "nothing posted");
        assert!(!running);
        assert!(status.contains("another song was opened"), "{status}");
    }

    /// [`app_with_a_stored_song`], whose song plays from its usable
    /// `mix.f32`, too long for the memory path.
    async fn app_playing_from_its_mix() -> (StemApp, js_sys::Array, String) {
        let (mut app, posts, id) = app_with_a_stored_song().await;
        app.memory_limit = ONE_SECOND;
        plant_mix(&id, &song_mix_file(), Some(FRAMES)).await;
        let song = app
            .song
            .as_ref()
            .expect("a song")
            .as_song()
            .expect("in memory");
        let file = store_meta::LaneFile::f32(store::mix_path(&id), FRAMES);
        app.load_song(Shown::with_files(song, vec![Some(file)]));
        (app, posts, id)
    }

    /// What the session's mix is now: its `mix` line, whether `mix.f32` is
    /// there, and whether its removal waits.
    async fn mix_state(id: &str) -> (Option<usize>, bool, bool) {
        store::test_settled(id).await;
        let line = meta_of(id).await.mix.map(|(f, _)| f);
        let file = store::test_read(&store::mix_path(id)).await.is_some();
        (line, file, store::mix_removal_waits(id))
    }

    /// D10 of M6: after `unstored` for the shown song that plays from
    /// `mix.f32`, the mix and its line stay, and a second Separate posts
    /// the same mix and cancels the removal.
    #[wasm_bindgen_test]
    async fn unstored_keeps_the_mix_that_playback_reads() {
        let (mut app, posts, id) = app_playing_from_its_mix().await;
        let protected = store::protected(&id);
        press_separate(&mut app).await;
        deliver_unstored(&mut app, "QuotaExceededError: the disk is full");
        let kept = mix_state(&id).await;
        let status = app.status.clone();
        posts.set_length(0);
        press_separate(&mut app).await;
        let sent = sessions(&posts);
        let again = mix_state(&id).await;
        finish(app, &id).await;
        assert!(protected, "the song plays from its session");
        assert_eq!(kept, (Some(FRAMES), true, true), "kept; the removal waits");
        assert!(
            status.contains("open another song, then open this one again and press Separate"),
            "{status}"
        );
        assert_eq!(sent, [Some(store::session_folder(&id))], "the storage path");
        assert_eq!(
            again,
            (Some(FRAMES), true, false),
            "the removal is cancelled"
        );
    }

    /// D10 of M6: `unstored`, then another song opens with no second
    /// Separate: the mix goes, as D6 of M4 says.
    #[wasm_bindgen_test]
    async fn unstored_then_another_song_removes_the_mix() {
        let (mut app, _posts, id) = app_playing_from_its_mix().await;
        press_separate(&mut app).await;
        deliver_unstored(&mut app, "QuotaExceededError: the disk is full");
        let kept = mix_state(&id).await;
        app.open_memory_song(Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![Stem {
                name: "lloyd".into(),
                left: vec![0.1; 4].into(),
                right: vec![0.1; 4].into(),
            }],
        });
        let after = mix_state(&id).await;
        finish(app, &id).await;
        assert_eq!(kept, (Some(FRAMES), true, true));
        assert_eq!(after, (None, false, false), "the mix goes");
    }

    /// D10 of M6: a second Separate, then another song opens while it runs:
    /// the mix stays until the job ends, and after a job that succeeds.
    #[wasm_bindgen_test]
    async fn a_mix_posted_again_stays_while_its_job_runs() {
        let (mut app, _posts, id) = app_playing_from_its_mix().await;
        press_separate(&mut app).await;
        deliver_unstored(&mut app, "QuotaExceededError: the disk is full");
        press_separate(&mut app).await;
        app.publish_separating();
        app.open_memory_song(Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![Stem {
                name: "lloyd".into(),
                left: vec![0.1; 4].into(),
                right: vec![0.1; 4].into(),
            }],
        });
        app.publish_separating();
        let while_running = mix_state(&id).await;
        plant_parts(&id, &NAMES).await;
        deliver_stored_and_done(&mut app);
        settle(&mut app).await;
        app.publish_separating();
        let after = mix_state(&id).await;
        finish(app, &id).await;
        assert_eq!(while_running, (Some(FRAMES), true, false));
        assert_eq!(after, (Some(FRAMES), true, false), "kept after a success");
    }
}

/// Browser tests of the page's open and restore through the decode worker
/// (D9 to D12 of `plans/active/stream-from-storage-m5.md`), with a fake
/// decode worker: a spy keeps what the page posts, and the test plants the
/// worker's part files and delivers its replies.
#[cfg(all(test, target_arch = "wasm32"))]
mod decode_tests;

/// Browser tests of the stored lanes (D7 to D10 of
/// `plans/active/stream-from-storage-m6.md`): the reader fills the read-ahead
/// buffer from session files that the test plants.
#[cfg(all(test, target_arch = "wasm32"))]
mod playback_tests;
