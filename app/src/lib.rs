//! Stem explorer: waveform lanes for a song and its stems, with mute, solo,
//! volume, tempo, a loop region and click-to-seek.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use engine::{Player, PlayerConfig, SharedParams, Song, Stem, StretcherKind};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

#[cfg(target_arch = "wasm32")]
pub mod picker;
pub mod reset;
#[cfg(target_arch = "wasm32")]
pub mod separator;
pub mod spectrogram;
#[cfg(target_arch = "wasm32")]
pub mod store;
pub mod store_meta;

/// Lanes that are not parts of the mix: computed from the others, or the
/// original recording kept beside its separated stems.  They start muted
/// and are left out of the mix lane.
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
    let lefts: Vec<Vec<f32>> = src.iter().map(|s| s.left.clone()).collect();
    let rights: Vec<Vec<f32>> = src.iter().map(|s| s.right.clone()).collect();
    (dsp::mix(&lefts, &volumes), dsp::mix(&rights, &volumes))
}

/// A separated song: the separated stems, then the source's mix as an
/// "original" lane (the source's own "original" lane when it has one).
pub fn with_original(source: &Song, mut stems: Vec<Stem>) -> Song {
    let original = match source.stem_index("original") {
        Some(i) => source.stems[i].clone(),
        None => {
            let (left, right) = mix_of_sources(source);
            Stem {
                name: "original".into(),
                left,
                right,
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
}

/// A song to show, with what belongs to it, applied together so the shown
/// song never takes another song's session or name.  Web only.
#[cfg(target_arch = "wasm32")]
struct Opened {
    song: Song,
    /// The browser-storage session it is kept in.
    session: Option<String>,
    /// Its name, for downloads and messages.
    name: Option<String>,
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

/// The app's source and where the model comes from, with links: shown in the transport bar.
#[cfg(target_arch = "wasm32")]
pub fn about_menu(ui: &mut egui::Ui) {
    ui.menu_button("About", |ui| {
        ui.set_max_width(420.0);
        ui.hyperlink_to(
            "Source code (GitHub): lloydmeta/stemyla",
            "https://github.com/lloydmeta/stemyla",
        );
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
    });
}

#[cfg(not(target_arch = "wasm32"))]
const WELCOME: &str = "Drop a folder of stems (or several WAV/MP3 files), or press Demo song.";
#[cfg(target_arch = "wasm32")]
const WELCOME: &str =
    "Press Open song… (or drop a song, or several WAV/MP3 stems, on the page), or press Demo song.";

/// What the song picker offers: the formats `engine` decodes, as
/// extensions, since iOS Safari treats `audio/*` as video (WebKit bug
/// 242110).
#[cfg(target_arch = "wasm32")]
const SONG_FILES: &str = ".mp3,.wav";

/// Decodes files the user chose and keeps them in browser storage, so a
/// reload brings the song back.  The result arrives through the inbox.
#[cfg(target_arch = "wasm32")]
async fn load_and_keep(
    files: Vec<(String, Vec<u8>)>,
    pending: std::rc::Rc<std::cell::RefCell<Inbox>>,
    ctx: egui::Context,
) {
    let name = files.first().map(|(n, _)| store_meta::file_title(n));
    let opened = match Song::load_files(files.clone()) {
        Ok(song) => {
            let session = match store::save_sources(&files, song.sample_rate).await {
                Ok(id) => Some(id),
                Err(e) => {
                    log::warn!("song not kept in browser storage: {e:#}");
                    None
                }
            };
            if let Some(id) = &session {
                log::info!("song kept in browser storage as session {id}");
            }
            Ok(Opened {
                song,
                session,
                name,
            })
        }
        Err(e) => Err(e),
    };
    pending.borrow_mut().song = Some(opened);
    ctx.request_repaint();
    refresh_storage(&pending).await;
}

/// Columns of peak data kept per lane; drawing re-aggregates to pixels.
const COLUMNS: usize = 4096;
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
    /// A real device through cpal; the callback owns the player.
    #[cfg(feature = "audio")]
    Live(engine::live::LiveOutput),
    /// No device: the UI renders the player silently so the playhead still
    /// moves.  Used in tests, in headless runs, and if the device fails.
    Silent(Box<Player>),
}

pub struct StemApp {
    pub song: Option<Arc<Song>>,
    /// Counts the songs opened (not lanes added to the shown one), so work
    /// that finishes later can tell whether its song is still the one shown.
    song_serial: u64,
    pub params: Option<Arc<SharedParams>>,
    pub lanes: Vec<Lane>,
    /// Screen rectangles of the lanes drawn in the last frame (mix first).
    pub lane_rects: Vec<Rect>,
    pub loop_bar_rect: Option<Rect>,
    pub status: String,
    pub output: Option<Output>,
    pub tempo_pct: f32,
    pub show_spectrograms: bool,
    /// Ask for a real audio device when a song is loaded.
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
    #[cfg(target_arch = "wasm32")]
    pending: std::rc::Rc<std::cell::RefCell<Inbox>>,
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
    /// The stem separation worker (web only).
    #[cfg(target_arch = "wasm32")]
    separator: Option<separator::Separator>,
    /// The song the running separation was started on.
    #[cfg(target_arch = "wasm32")]
    separating: Option<SeparationJob>,
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
    stems_for_opening: Option<(String, std::rc::Rc<Vec<Stem>>, String)>,
    #[cfg(target_arch = "wasm32")]
    reset: ResetStage,
}

/// A classic-drums job: the serial of the song it started on, and where
/// its lane arrives.
#[cfg(not(target_arch = "wasm32"))]
type ClassicJob = (u64, std::sync::mpsc::Receiver<(Vec<f32>, Vec<f32>)>);

/// The song a separation was started on, so its stems go to that song even
/// if another is opened while it runs.
pub struct SeparationJob {
    serial: u64,
    /// The song's browser-storage session, where its stems are kept.
    session: Option<String>,
    name: Option<String>,
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
#[cfg(target_arch = "wasm32")]
const SEPARATION_RATE: u32 = 44_100;

impl Default for StemApp {
    fn default() -> Self {
        Self {
            song: None,
            song_serial: 0,
            params: None,
            lanes: Vec::new(),
            lane_rects: Vec::new(),
            loop_bar_rect: None,
            status: WELCOME.into(),
            output: None,
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
            pending: Default::default(),
            #[cfg(target_arch = "wasm32")]
            session: None,
            #[cfg(target_arch = "wasm32")]
            usage: (0.0, 0.0),
            #[cfg(target_arch = "wasm32")]
            song_name: None,
            #[cfg(target_arch = "wasm32")]
            sessions: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            separator: None,
            #[cfg(target_arch = "wasm32")]
            separating: None,
            #[cfg(target_arch = "wasm32")]
            opening: None,
            #[cfg(target_arch = "wasm32")]
            opens: 0,
            #[cfg(target_arch = "wasm32")]
            stems_for_opening: None,
            #[cfg(target_arch = "wasm32")]
            reset: ResetStage::Closed,
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

/// Peak columns for a stereo signal: `dsp::peaks` on each channel, then the
/// lower minimum and higher maximum of the two.
pub fn lane_columns(left: &[f32], right: &[f32]) -> (Vec<(f32, f32)>, f32) {
    let block = left.len().div_ceil(COLUMNS).max(1);
    let pl = dsp::peaks(left, block);
    let pr = dsp::peaks(right, block);
    let cols: Vec<(f32, f32)> = pl
        .iter()
        .zip(&pr)
        .map(|(a, b)| (a.0.min(b.0), a.1.max(b.1)))
        .collect();
    let magnitudes: Vec<f32> = cols.iter().map(|(lo, hi)| lo.abs().max(hi.abs())).collect();
    let loudest = if magnitudes.is_empty() {
        0.0
    } else {
        dsp::biggest(&magnitudes)
    };
    let scale = if loudest > 1e-6 { 1.0 / loudest } else { 1.0 };
    (cols, scale)
}

impl StemApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Same look everywhere (the browser would otherwise follow the OS).
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        #[allow(unused_mut)]
        let mut app = Self::default();
        #[cfg(target_arch = "wasm32")]
        match separator::Separator::new(cc.egui_ctx.clone()) {
            Ok(s) => app.separator = Some(s),
            Err(e) => log::warn!("stem separation unavailable: {e}"),
        }
        #[cfg(target_arch = "wasm32")]
        {
            // Bring back the song (and stems) kept from the last visit.
            let (pending, ctx) = (app.pending.clone(), cc.egui_ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(r) = store::restore().await {
                    let mut inbox = pending.borrow_mut();
                    match r {
                        Ok((id, song, meta)) => {
                            let what = if meta.stems.is_empty() {
                                ""
                            } else {
                                " and its separated stems"
                            };
                            let text =
                                format!("Restored \"{}\"{what} from this browser.", meta.name);
                            log::info!("{text}");
                            inbox.status = Some(text);
                            inbox.song = Some(Ok(Opened {
                                song,
                                session: Some(id),
                                name: Some(meta.name.clone()),
                            }));
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

    /// Shows a different song.  [`Self::load_song`] alone is for the shown
    /// song with lanes added or replaced.
    pub fn open_song(&mut self, song: Song) {
        self.song_serial += 1;
        self.load_song(song);
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
        })
    }

    /// Gives a finished separation's stems to the song it was started on:
    /// kept in browser storage with that song, and shown if it is still the
    /// open song.
    pub fn apply_separated(&mut self, job: SeparationJob, stems: Vec<Stem>, secs: f64) {
        let n = stems.len();
        log::info!("separator: {n} stems in {secs:.1} s");
        #[cfg(target_arch = "wasm32")]
        let shown_session = self.session.as_deref();
        #[cfg(not(target_arch = "wasm32"))]
        let shown_session = None;
        let destination = job.destination(self.song_serial, shown_session);
        let name = job
            .name
            .clone()
            .unwrap_or_else(|| "the separated song".into());
        // Shared by the save and the held copy rather than copied for each:
        // six stems of a song run to hundreds of MB.
        let stems = std::rc::Rc::new(stems);
        #[cfg(target_arch = "wasm32")]
        if let Some(id) = job.session.clone() {
            let kept_text = format!(
                "Separated \"{name}\" into {n} stems in {secs:.0} s and kept them in this browser; open it from Storage to play them."
            );
            let being_opened = self
                .opening
                .as_ref()
                .is_some_and(|(_, opening)| *opening == id);
            let kept = (destination == StemsGo::Kept && !being_opened).then(|| kept_text.clone());
            if destination == StemsGo::Kept && being_opened {
                self.stems_for_opening = Some((id.clone(), stems.clone(), kept_text));
            }
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
        let song = match (destination, self.song.clone()) {
            (StemsGo::Show, Some(song)) => song,
            (StemsGo::Kept, _) => {
                self.status =
                    format!("Separated \"{name}\" into {n} stems; keeping them in this browser…");
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
        self.load_song(with_original(&song, std::rc::Rc::unwrap_or_clone(stems)));
        if let (Some(h), Some(p)) = (head, &self.params) {
            p.request_seek(h);
        }
        self.status = format!("Separated into {n} stems in {secs:.0} s.  {}", self.status);
    }

    /// Loads a song: builds lanes, shared params and the output.
    pub fn load_song(&mut self, song: Song) {
        let song = Arc::new(song);
        let mut lanes = Vec::with_capacity(song.stems.len() + 1);
        let (ml, mr) = mix_of_sources(&song);
        let (cols, scale) = lane_columns(&ml, &mr);
        lanes.push(Lane {
            name: "mix".into(),
            stem: None,
            color: Color32::from_rgb(220, 220, 220),
            columns: cols,
            scale,
            spectrogram: None,
        });
        for (i, s) in song.stems.iter().enumerate() {
            let (cols, scale) = lane_columns(&s.left, &s.right);
            lanes.push(Lane {
                name: s.name.clone(),
                stem: Some(i),
                color: stem_color(&s.name),
                columns: cols,
                scale,
                spectrogram: None,
            });
        }
        self.lanes = lanes;
        let params = Arc::new(SharedParams::new(song.stems.len()));
        params.set_tempo(self.tempo_pct / 100.0);
        for (i, s) in song.stems.iter().enumerate() {
            if DERIVED.contains(&s.name.as_str()) {
                params.set_muted(i, true);
            }
        }
        self.output = None; // stops any previous stream
        let player = Player::with_config(
            Arc::clone(&song),
            Arc::clone(&params),
            self.stretcher.build(song.sample_rate),
            PlayerConfig::default(),
        );
        self.status = format!(
            "{} stems, {:.1} s at {} Hz",
            song.stems.len(),
            song.duration_s(),
            song.sample_rate
        );
        self.song = Some(song);
        self.params = Some(params);
        self.output = Some(self.open_output(player));
    }

    fn open_output(&mut self, player: Player) -> Output {
        #[cfg(feature = "audio")]
        if self.want_device {
            // cpal's start() takes the player; on failure we never get it
            // back, so probe first.
            match engine::live::probe_default_output() {
                Ok(_) => match engine::live::start(player) {
                    Ok(out) => {
                        self.status
                            .push_str(&format!(", playing on {}", out.device_name));
                        return Output::Live(out);
                    }
                    Err(e) => {
                        self.status.push_str(&format!(", audio failed: {e:#}"));
                        return Output::Silent(Box::new(self.rebuild_player()));
                    }
                },
                Err(e) => self
                    .status
                    .push_str(&format!(", no audio device ({e:#}); silent")),
            }
        }
        Output::Silent(Box::new(player))
    }

    /// Only used when a device fails after the player was handed over.
    #[cfg(feature = "audio")]
    fn rebuild_player(&self) -> Player {
        let song = self.song.clone().expect("song loaded");
        let params = self.params.clone().expect("params");
        Player::with_config(
            Arc::clone(&song),
            params,
            self.stretcher.build(song.sample_rate),
            PlayerConfig::default(),
        )
    }

    /// Loads stems from in-memory files (a multi-file drop).
    pub fn load_files(&mut self, files: Vec<(String, Vec<u8>)>) {
        match Song::load_files(files) {
            Ok(s) => self.open_song(s),
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
                    left: st.left,
                    right: st.right,
                })
                .collect(),
        };
        self.open_song(song);
    }

    /// Adds an extra stem lane (for example "classic drums") and reloads.
    pub fn add_stem(&mut self, name: &str, left: Vec<f32>, right: Vec<f32>) {
        if let Some(song) = &self.song {
            let mut s = (**song).clone();
            s.stems.retain(|st| st.name != name);
            s.stems.push(Stem {
                name: name.into(),
                left,
                right,
            });
            self.load_song(s);
        }
    }

    /// Starts HPSS on the mix.  Natively it runs on a thread and the lane
    /// appears when it finishes; on the web it blocks the page while it runs.
    pub fn start_classic_drums(&mut self) {
        let Some(song) = self.song.clone() else {
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
            self.classic_job = Some((self.song_serial, rx));
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
            false
        }
    }

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((serial, rx)) = &self.classic_job {
            match rx.try_recv() {
                Ok(_) if *serial != self.song_serial => {
                    self.classic_job = None;
                    self.status =
                        "Classic drums finished for a song that is no longer open; discarded"
                            .into();
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
    }

    /// Advances the silent output by the frame time so the playhead moves.
    fn pump_silent(&mut self, dt: f32) {
        let Some(Output::Silent(player)) = &mut self.output else {
            return;
        };
        let sr = player.song().sample_rate as f32;
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
                    Ok(s) => self.open_song(s),
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
            self.status = format!("Reading {} files…", dropped.len());
            wasm_bindgen_futures::spawn_local(async move {
                let mut files = Vec::new();
                for f in &dropped {
                    match f.bytes_async().await {
                        Ok(b) => files.push((f.path().to_string_lossy().into_owned(), b)),
                        Err(e) => {
                            pending.borrow_mut().song = Some(Err(anyhow::anyhow!(e)));
                            ctx.request_repaint();
                            return;
                        }
                    }
                }
                load_and_keep(files, pending, ctx).await;
            });
        }
    }

    /// "Open song…": the browser's file picker, the way to load a song on
    /// a phone or tablet.  Several files load as stems of one song, as a
    /// multi-file drop does.
    #[cfg(target_arch = "wasm32")]
    fn pick_song(&mut self, ctx: &egui::Context) {
        let (pending, ctx) = (self.pending.clone(), ctx.clone());
        picker::pick_files(SONG_FILES, true, move |chosen| {
            if chosen.is_empty() {
                return;
            }
            {
                let mut inbox = pending.borrow_mut();
                inbox.status = Some(format!("Reading {} files…", chosen.len()));
            }
            ctx.request_repaint();
            wasm_bindgen_futures::spawn_local(async move {
                let mut files = Vec::new();
                // The "Reading…" text may still be queued; the result,
                // applied before the queued status, must not be hidden by it.
                for f in &chosen {
                    match picker::read(f).await {
                        Ok(b) => files.push((f.name(), b)),
                        Err(e) => {
                            let mut inbox = pending.borrow_mut();
                            inbox.status = None;
                            inbox.song = Some(Err(anyhow::anyhow!(e)));
                            drop(inbox);
                            ctx.request_repaint();
                            return;
                        }
                    }
                }
                pending.borrow_mut().status = None;
                load_and_keep(files, pending, ctx).await;
            });
        });
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            #[cfg(target_arch = "wasm32")]
            if ui
                .button("Open song…")
                .on_hover_text("Choose an MP3 or WAV file, or several stems of one song")
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
                    .add_enabled(!self.classic_running(), egui::Button::new("Classic drums"))
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
        let Some(song) = self.song.clone() else {
            return;
        };
        if ui
            .button("Separate 6 stems")
            .on_hover_text("BS-RoFormer SW on the GPU: bass, drums, other, vocals, guitar, piano")
            .clicked()
        {
            if song.sample_rate != SEPARATION_RATE {
                self.status = format!(
                    "Separation needs 44.1 kHz audio; this song is {} Hz",
                    song.sample_rate
                );
                return;
            }
            let (l, r) = match song.stem_index("original") {
                Some(i) => (song.stems[i].left.clone(), song.stems[i].right.clone()),
                None => mix_of_sources(&song),
            };
            if sep.separate(&l, &r) {
                self.separating = self.separation_job();
                self.status = "Separating…".into();
            } else {
                self.status = "Could not reach the separation worker".into();
            }
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
                    if ui.small_button("Forget").on_hover_text("Delete this song and its stems from the browser").clicked() {
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
            wasm_bindgen_futures::spawn_local(async move {
                let Some(r) = store::restore_id(&id).await else {
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
                        Ok((id, song, meta)) => {
                            inbox.status =
                                Some(format!("Opened \"{}\" from this browser.", meta.name));
                            inbox.song = Some(Ok(Opened {
                                song,
                                session: Some(id),
                                name: Some(meta.name),
                            }));
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

    /// Stops playback and the worker, then runs the reset; the page reloads
    /// when it succeeds.
    #[cfg(target_arch = "wasm32")]
    fn start_reset(&mut self, ctx: &egui::Context, choice: reset::Reset) {
        self.reset = ResetStage::Running;
        self.output = None;
        if let Some(sep) = &mut self.separator {
            sep.stop();
        }
        let (pending, ctx) = (self.pending.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(failure) = reset::run(choice).await {
                log::warn!("reset: {failure}");
                pending.borrow_mut().reset_failed = Some((choice, failure));
                ctx.request_repaint();
            }
        });
    }

    /// Applies messages from the separation worker.
    #[cfg(target_arch = "wasm32")]
    fn poll_separator(&mut self) {
        use separator::Event;
        let Some(sep) = &mut self.separator else {
            return;
        };
        let mut finished = None;
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
                    self.status = format!("Saved {name}.");
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
                    finished = Some((std::mem::take(&mut sep.stems), secs));
                }
                Event::Error { of, text } => {
                    log::error!("separator: {of}: {text}");
                    let what = match of.as_str() {
                        "separate" => {
                            sep.running = false;
                            self.separating = None;
                            "Separation"
                        }
                        "mp3" => "MP3 encoding",
                        "selftest" => "Self-test",
                        "bench" => "Benchmark",
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
                .map(|(name, left, right)| Stem { name, left, right })
                .collect();
            match self.separating.take() {
                Some(job) => self.apply_separated(job, stems, secs),
                None => log::warn!("separator: stems arrived with no separation recorded"),
            }
        }
    }

    fn lane_controls(&mut self, ui: &mut egui::Ui, lane: usize) {
        let params = self.params.clone().expect("params");
        let l = &self.lanes[lane];
        ui.set_width(200.0);
        ui.horizontal(|ui| {
            ui.colored_label(l.color, egui::RichText::new(&l.name).strong());
            #[cfg(target_arch = "wasm32")]
            if let (Some(stem), Some(song)) = (l.stem, self.song.clone()) {
                let s = &song.stems[stem];
                let base = format!(
                    "{} - {}",
                    self.song_name.as_deref().unwrap_or("song"),
                    s.name
                );
                let mut status = None;
                ui.menu_button("Save", |ui| {
                    if ui.button("WAV (16-bit, lossless)").clicked() {
                        store::download(
                            &format!("{base}.wav"),
                            &engine::wav::encode_pcm16(&s.left, &s.right, song.sample_rate),
                            "audio/wav",
                        );
                        ui.close();
                    }
                    if ui.button("MP3 (192 kbps)").clicked() {
                        if let Some(sep) = &self.separator {
                            sep.encode_mp3(
                                &format!("{base}.mp3"),
                                &s.left,
                                &s.right,
                                song.sample_rate,
                            );
                            status = Some(format!("Encoding {base}.mp3…"));
                        }
                        ui.close();
                    }
                });
                if let Some(text) = status {
                    self.status = text;
                }
            }
        });
        let Some(stem) = l.stem else {
            ui.small("sum of all stems");
            return;
        };
        ui.horizontal(|ui| {
            let mut m = params.is_muted(stem);
            if ui.toggle_value(&mut m, "Mute").changed() {
                params.set_muted(stem, m);
            }
            let mut s = params.is_soloed(stem);
            if ui.toggle_value(&mut s, "Solo").changed() {
                params.set_soloed(stem, s);
            }
            let mut v = params.stems[stem].volume.get();
            if ui
                .add(egui::Slider::new(&mut v, 0.0..=1.5).show_value(false))
                .changed()
            {
                params.set_volume(stem, v);
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

        let any_solo = (0..song.stems.len()).any(|i| params.is_soloed(i));
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
        self.poll_separator();
        #[cfg(target_arch = "wasm32")]
        {
            let inbox = std::mem::take(&mut *self.pending.borrow_mut());
            if let Some(r) = inbox.song {
                match r {
                    Ok(opened) => {
                        let holds_these = self
                            .stems_for_opening
                            .as_ref()
                            .is_some_and(|(id, ..)| opened.session.as_deref() == Some(id.as_str()));
                        let song = match self.stems_for_opening.take_if(|_| holds_these) {
                            Some((_, stems, _)) => {
                                with_original(&opened.song, std::rc::Rc::unwrap_or_clone(stems))
                            }
                            None => opened.song,
                        };
                        self.session = opened.session;
                        self.song_name = opened.name;
                        self.open_song(song);
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
        }
        let dt = ctx.input(|i| i.stable_dt);
        self.pump_silent(dt);
        self.log_audio_stats(dt);
        if self.show_spectrograms {
            spectrogram::ensure_textures(&ctx, &mut self.lanes, self.song.as_deref());
        }

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
    }
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

    fn job(serial: u64, session: Option<&str>) -> SeparationJob {
        SeparationJob {
            serial,
            session: session.map(String::from),
            name: None,
        }
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
