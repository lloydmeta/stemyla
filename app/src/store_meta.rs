//! The `meta` file of a stored song session (see `store`): plain
//! tab-separated lines, readable without the browser.  A line of a kind
//! that this build does not know is ignored.

/// The format of the mix file that this build writes: raw 32-bit
/// little-endian floats, interleaved stereo, no header, at 44.1 kHz (D1 of
/// M4 in `plans/active/stream-from-storage-m4.md`).  A build that changes
/// the samples or the layout uses a new tag, so it writes the mix again.
pub const MIX_FORMAT: &str = "f32le-1";

/// What a session holds, from its `meta` file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub name: String,
    pub rate: u32,
    /// (original file name, stored file name)
    pub files: Vec<(String, String)>,
    pub stems: Vec<String>,
    /// The `mix` line: the frames and the format of the session's complete
    /// mix file, `mix.f32`.
    pub mix: Option<(usize, String)>,
    /// The `decoded` line: the session files of the decode worker are
    /// complete (D2 of `plans/active/stream-from-storage-m5.md`).
    pub decoded: Option<Decoded>,
    /// The `stems-generation` line: one more for each change of the `stem`
    /// lines, so two separations, whose stems have the same names, differ
    /// (D7 of `plans/active/stream-from-storage-m7.md`).  0 when there is
    /// no line.
    pub stems_generation: u64,
    /// The `stem-overviews` line: the overview files of the stems are
    /// complete, for these frames and this generation of the stems.
    pub stem_overviews: Option<StemOverviews>,
}

/// The `stem-overviews` line of `meta` (D7 of the M7 plan).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StemOverviews {
    pub frames: usize,
    pub format: String,
    pub generation: u64,
}

/// The overview file of the stem of the `k`th `stem` line, from the
/// session folder.
pub fn stem_overview_file(k: usize) -> String {
    format!("overviews/stems/{k}.f32")
}

/// The overview file of the mix of the stems, from the session folder.
pub const STEM_MIX_OVERVIEW: &str = "overviews/stems/mix.f32";

/// The part file that the decode worker writes for `file`, an overview of
/// the stems of `generation`.  Two requests for different generations of
/// one session never share a part file.  Two requests of one generation
/// can: they write the same bytes, and an overview file of the wrong size
/// is refused (D4 of the M7 plan).
pub fn stem_overview_part(file: &str, generation: u64) -> String {
    format!("{file}.{generation}.decode.part")
}

/// The bytes of an overview file of a lane of `frames` frames: 8 for each
/// column (D4 of the M7 plan).
pub fn overview_bytes(frames: usize) -> u64 {
    8 * engine::overview::column_count(frames) as u64
}

/// The `decoded` line: the song's frames, the format of its session files
/// and overviews, and the channels (1 or 2) of each source, by the index
/// of its stored file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decoded {
    pub frames: usize,
    pub format: String,
    pub channels: Vec<(u32, u8)>,
}

/// A source of a stored song, as the decode worker takes it: in the order
/// of `Song::from_audio`.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedSource {
    /// The `<i>` of its stored file `src/<i>.<ext>`.
    pub index: u32,
    /// Its stem name, such as `"drums"`.
    pub name: String,
    /// Its stored file, from the session folder: `src/<i>.<ext>`.
    pub file: String,
    /// The extension of its original name, for the format probe.
    pub ext: String,
    /// False for a derived source, such as `classic drums`.
    pub in_mix: bool,
}

/// The formats of the lanes that an open of `meta`'s song shows, and one
/// 16-bit lane for classic drums (D2 of the M7 plan): for a session with
/// `stem` lines, its stems (16-bit) and the original (f32); else each
/// source as an f32 lane.
pub fn open_formats(meta: &Meta) -> Vec<engine::readahead::LaneFormat> {
    use engine::readahead::LaneFormat::{F32, Pcm16};
    let mut formats = if meta.stems.is_empty() {
        let sources = planned_sources(meta).map_or(meta.files.len(), |s| s.len());
        vec![F32; sources]
    } else {
        let mut stems = vec![Pcm16; meta.stems.len()];
        stems.push(F32);
        stems
    };
    formats.push(Pcm16);
    formats
}

/// Whether lanes of `formats` fit the read-ahead buffer at its minimum
/// slots (D2 of the M7 plan).
pub fn fits_read_ahead(formats: &[engine::readahead::LaneFormat]) -> bool {
    engine::readahead::ReadAhead::slot_counts(formats).is_some()
}

/// The sources of `meta`'s files, in the order of `Song::from_audio`:
/// files that are not stems (`mix`, `original`, `truth`) left out, and the
/// rest sorted by `engine::song::rank`, keeping the order of equal names.
/// An error if a stored file is not named `<i>.<ext>`.
pub fn planned_sources(meta: &Meta) -> Result<Vec<PlannedSource>, String> {
    let mut sources = Vec::new();
    for (orig, stored) in &meta.files {
        let name = engine::song::stem_name(orig);
        if !engine::song::is_stem(&name) {
            continue;
        }
        let index = stored
            .split_once('.')
            .and_then(|(i, _)| i.parse().ok())
            .ok_or_else(|| format!("the stored file {stored} is not named <i>.<ext>"))?;
        // As `Song::load_files` gives the decoder its hint.
        let ext = orig
            .rsplit('.')
            .next()
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        sources.push(PlannedSource {
            index,
            in_mix: !crate::DERIVED.contains(&name.as_str()),
            name,
            file: format!("src/{stored}"),
            ext,
        });
    }
    sources.sort_by_key(|s| engine::song::rank(&s.name));
    Ok(sources)
}

/// Whether the song of `sources` is a song of one source: one source in the
/// mix, whose file is `mix.f32` (D1).
pub fn one_source(sources: &[PlannedSource]) -> bool {
    sources.iter().filter(|s| s.in_mix).count() == 1
}

/// The session file of `source`, from the session folder: `mix.f32` for
/// the source in the mix of a song of one source (`one`), else
/// `decoded/<i>.f32`.
pub fn source_file(source: &PlannedSource, one: bool) -> String {
    if one && source.in_mix {
        "mix.f32".into()
    } else {
        format!("decoded/{}.f32", source.index)
    }
}

/// Every session file of D1 for a song of `frames` frames, with the size
/// that a complete file has: 8 bytes a frame for `mix.f32` and each
/// `decoded/<i>.f32`, and 8 bytes a column for each overview.
pub fn session_files(frames: usize, sources: &[PlannedSource]) -> Vec<(String, u64)> {
    let one = one_source(sources);
    let samples = 8 * frames as u64;
    let overview = 8 * engine::overview::column_count(frames) as u64;
    let mut out = vec![("mix.f32".to_string(), samples)];
    for source in sources {
        let file = source_file(source, one);
        if file != "mix.f32" {
            out.push((file, samples));
        }
    }
    for source in sources {
        out.push((format!("overviews/{}.f32", source.index), overview));
    }
    out.push(("overviews/mix.f32".into(), overview));
    out
}

/// The frames of a complete session (D2): its `decoded` line has the format
/// [`MIX_FORMAT`], at least 1 frame and the channels of each source, and
/// each file of [`session_files`] has its size, as `size_of` gives it.
pub fn complete_frames(
    meta: &Meta,
    sources: &[PlannedSource],
    size_of: impl Fn(&str) -> Option<u64>,
) -> Option<usize> {
    let decoded = meta.decoded.as_ref()?;
    let usable = decoded.format == MIX_FORMAT
        && decoded.frames > 0
        && sources
            .iter()
            .all(|s| decoded.channels_of(s.index).is_some());
    let sized = session_files(decoded.frames, sources)
        .iter()
        .all(|(path, size)| size_of(path) == Some(*size));
    (usable && sized).then_some(decoded.frames)
}

impl Decoded {
    /// The channels of the source of stored index `index`.
    pub fn channels_of(&self, index: u32) -> Option<u8> {
        self.channels
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, c)| *c)
    }

    fn to_line(&self) -> String {
        let channels: Vec<String> = self
            .channels
            .iter()
            .map(|(i, c)| format!("{i}:{c}"))
            .collect();
        format!(
            "decoded\t{}\t{}\t{}\n",
            self.frames,
            clean(&self.format),
            channels.join(",")
        )
    }

    fn parse(frames: &str, format: &str, channels: &str) -> Option<Decoded> {
        let channels = channels
            .split(',')
            .filter(|c| !c.is_empty())
            .map(|entry| {
                let (i, c) = entry.split_once(':')?;
                let c: u8 = c.parse().ok().filter(|c| *c == 1 || *c == 2)?;
                Some((i.parse().ok()?, c))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Decoded {
            frames: frames.parse().ok()?,
            format: format.into(),
            channels,
        })
    }
}

impl Meta {
    pub fn to_text(&self) -> String {
        let mut s = format!("name\t{}\nrate\t{}\n", clean(&self.name), self.rate);
        for (orig, stored) in &self.files {
            s += &format!("file\t{}\t{stored}\n", clean(orig));
        }
        for stem in &self.stems {
            s += &format!("stem\t{}\n", clean(stem));
        }
        if let Some((frames, format)) = &self.mix {
            s += &format!("mix\t{frames}\t{}\n", clean(format));
        }
        if let Some(decoded) = &self.decoded {
            s += &decoded.to_line();
        }
        if self.stems_generation > 0 {
            s += &format!("stems-generation\t{}\n", self.stems_generation);
        }
        if let Some(o) = &self.stem_overviews {
            s += &format!(
                "stem-overviews\t{}\t{}\t{}\n",
                o.frames,
                clean(&o.format),
                o.generation
            );
        }
        s
    }

    /// New stems: the generation goes up, and the stem overviews, which
    /// were of the old stems, are no longer named (D7 of the M7 plan).
    pub fn new_stems(&mut self) {
        self.stems_generation += 1;
        self.stem_overviews = None;
    }

    /// Whether the `stem-overviews` line names the stems of now, a song of
    /// `frames` frames, and [`MIX_FORMAT`].  The files' sizes are checked
    /// apart.
    pub fn stem_overviews_for(&self, frames: usize) -> bool {
        self.stem_overviews.as_ref().is_some_and(|o| {
            o.frames == frames && o.format == MIX_FORMAT && o.generation == self.stems_generation
        })
    }

    /// The frames of the mix file, if `meta` names one in [`MIX_FORMAT`].
    pub fn usable_mix(&self) -> Option<usize> {
        match &self.mix {
            Some((frames, format)) if format == MIX_FORMAT => Some(*frames),
            _ => None,
        }
    }

    pub fn parse(text: &str) -> Meta {
        let mut m = Meta::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["name", n] => m.name = n.to_string(),
                ["rate", r] => m.rate = r.parse().unwrap_or(0),
                ["file", orig, stored] => m.files.push((orig.to_string(), stored.to_string())),
                ["stem", s] => m.stems.push(s.to_string()),
                ["mix", frames, format] => {
                    m.mix = frames.parse().ok().map(|f| (f, format.to_string()));
                }
                ["decoded", frames, format, channels] => {
                    m.decoded = Decoded::parse(frames, format, channels);
                }
                ["stems-generation", n] => m.stems_generation = n.parse().unwrap_or(0),
                ["stem-overviews", frames, format, generation] => {
                    m.stem_overviews = frames.parse().ok().zip(generation.parse().ok()).map(
                        |(frames, generation)| StemOverviews {
                            frames,
                            format: format.to_string(),
                            generation,
                        },
                    );
                }
                _ => {}
            }
        }
        m
    }
}

/// Tabs and newlines would break the line format.
fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// The last session asked to be current, for the writes of
/// `sessions/current` (`store::make_current`).  Each request is a task of
/// its own, so two writes of one page could end out of order: a restore of
/// A, then an open of B whose write ends first.  So one write runs at a
/// time.  A request that comes during a write waits for it, then writes
/// its session if it is still the last request.  The last request wins
/// (review rounds speed 5 and 6 of M8).
#[derive(Default)]
pub struct LastCurrent {
    state: std::cell::RefCell<CurrentState>,
}

#[derive(Default)]
struct CurrentState {
    /// The last session asked to be current.
    wanted: Option<String>,
    /// A write runs.
    writing: bool,
    /// The session of the last write, if it ended well.
    written: Option<String>,
    /// The requests that wait for the write to end.
    waiting: Vec<std::task::Waker>,
}

/// Resolves when no write of a [`LastCurrent`] runs.
struct Idle<'a>(&'a LastCurrent);

impl std::future::Future for Idle<'_> {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let mut state = self.0.state.borrow_mut();
        if !state.writing {
            return std::task::Poll::Ready(());
        }
        state.waiting.push(cx.waker().clone());
        std::task::Poll::Pending
    }
}

impl LastCurrent {
    /// Asks for `id` to be current, and writes it with `write`, which gives
    /// whether the write ended well.  It gives `true` only when no write
    /// runs and the last write named `id`, which is the last request.  A
    /// request that a later one replaced gives `false`, and so does a
    /// failed write.
    pub async fn make<F, W>(&self, id: &str, write: W) -> bool
    where
        W: Fn(String) -> F,
        F: std::future::Future<Output = bool>,
    {
        self.state.borrow_mut().wanted = Some(id.to_string());
        loop {
            Idle(self).await;
            {
                let mut state = self.state.borrow_mut();
                if state.wanted.as_deref() != Some(id) {
                    return false;
                }
                if state.written.as_deref() == Some(id) {
                    return true;
                }
                state.writing = true;
            }
            let ok = write(id.to_string()).await;
            let waiting = {
                let mut state = self.state.borrow_mut();
                state.writing = false;
                state.written = ok.then(|| id.to_string());
                std::mem::take(&mut state.waiting)
            };
            for waker in waiting {
                waker.wake();
            }
            if !ok {
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A write of `sessions/current` in a test: it ends when the test sets
    /// its gate, and when the gate says it ends well, the file holds its
    /// session.
    struct Write {
        id: String,
        gate: std::rc::Rc<std::cell::Cell<Option<bool>>>,
        file: std::rc::Rc<std::cell::RefCell<String>>,
    }

    impl std::future::Future for Write {
        type Output = bool;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<bool> {
            let Some(ok) = self.gate.get() else {
                return std::task::Poll::Pending;
            };
            if ok {
                *self.file.borrow_mut() = self.id.clone();
            }
            std::task::Poll::Ready(ok)
        }
    }

    type Gates =
        std::rc::Rc<std::cell::RefCell<Vec<(String, std::rc::Rc<std::cell::Cell<Option<bool>>>)>>>;

    /// A writer whose writes wait for their gates, which `gates` keeps in
    /// the order the writes start.
    fn writer(
        gates: &Gates,
        file: &std::rc::Rc<std::cell::RefCell<String>>,
    ) -> impl Fn(String) -> Write {
        let (gates, file) = (std::rc::Rc::clone(gates), std::rc::Rc::clone(file));
        move |id: String| {
            let gate = std::rc::Rc::new(std::cell::Cell::new(None));
            gates
                .borrow_mut()
                .push((id.clone(), std::rc::Rc::clone(&gate)));
            Write {
                id,
                gate,
                file: std::rc::Rc::clone(&file),
            }
        }
    }

    /// Review rounds speed 5 and 6 of M8: a restore of A writes
    /// `sessions/current`, then an open of B asks for B.  B's request waits
    /// for A's write, then writes B.  The file never names A after B's
    /// request is ready, and A's request gives `false`.
    #[test]
    fn the_last_session_asked_stays_current() {
        use std::task::{Context, Poll, Waker};
        let file = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let gates: Gates = std::rc::Rc::default();
        let last = LastCurrent::default();
        let mut cx = Context::from_waker(Waker::noop());
        let mut a = Box::pin(last.make("acme-a", writer(&gates, &file)));
        let mut b = Box::pin(last.make("acme-b", writer(&gates, &file)));
        assert!(a.as_mut().poll(&mut cx).is_pending());
        assert!(
            b.as_mut().poll(&mut cx).is_pending(),
            "B waits for A's write"
        );
        assert_eq!(gates.borrow().len(), 1, "one write at a time");
        gates.borrow()[0].1.set(Some(true));
        assert_eq!(a.as_mut().poll(&mut cx), Poll::Ready(false), "B replaced A");
        assert_eq!(*file.borrow(), "acme-a");
        assert!(b.as_mut().poll(&mut cx).is_pending(), "B writes B");
        assert_eq!(gates.borrow()[1].0, "acme-b");
        gates.borrow()[1].1.set(Some(true));
        assert_eq!(b.as_mut().poll(&mut cx), Poll::Ready(true));
        assert_eq!(*file.borrow(), "acme-b", "the last request wins");
        assert_eq!(gates.borrow().len(), 2, "no write after B's");
    }

    /// Review round speed 6 of M8: a failed write gives no `true`.  B's
    /// write fails after A's: neither request gives `true`, and a later
    /// request of B writes again.
    #[test]
    fn a_failed_write_of_the_current_session_gives_no_true() {
        use std::task::{Context, Poll, Waker};
        let file = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let gates: Gates = std::rc::Rc::default();
        let last = LastCurrent::default();
        let mut cx = Context::from_waker(Waker::noop());
        let mut a = Box::pin(last.make("acme-a", writer(&gates, &file)));
        let mut b = Box::pin(last.make("acme-b", writer(&gates, &file)));
        assert!(a.as_mut().poll(&mut cx).is_pending());
        assert!(b.as_mut().poll(&mut cx).is_pending());
        gates.borrow()[0].1.set(Some(true));
        assert_eq!(a.as_mut().poll(&mut cx), Poll::Ready(false));
        assert!(b.as_mut().poll(&mut cx).is_pending());
        gates.borrow()[1].1.set(Some(false));
        assert_eq!(
            b.as_mut().poll(&mut cx),
            Poll::Ready(false),
            "B's write failed"
        );
        assert_eq!(*file.borrow(), "acme-a", "the file keeps A");
        let mut again = Box::pin(last.make("acme-b", writer(&gates, &file)));
        assert!(again.as_mut().poll(&mut cx).is_pending(), "B writes again");
        gates.borrow()[2].1.set(Some(true));
        assert_eq!(again.as_mut().poll(&mut cx), Poll::Ready(true));
        assert_eq!(*file.borrow(), "acme-b");
    }

    /// D7 of the M7 plan: the `stems-generation` and `stem-overviews` lines
    /// read back, and the overviews are complete only for the stems of
    /// now and the song's frames.
    #[test]
    fn stem_overviews_name_one_generation_of_the_stems() {
        let mut m = Meta {
            name: "lloyd".into(),
            stems: vec!["bass".into(), "drums".into()],
            ..Meta::default()
        };
        assert!(!m.stem_overviews_for(100), "no line");
        m.new_stems();
        assert_eq!((m.stems_generation, m.stem_overviews.clone()), (1, None));
        m.stem_overviews = Some(StemOverviews {
            frames: 100,
            format: MIX_FORMAT.into(),
            generation: 1,
        });
        let back = Meta::parse(&m.to_text());
        assert_eq!(back, m, "the lines read back");
        assert!(back.stem_overviews_for(100));
        assert!(!back.stem_overviews_for(99), "other frames");
        let mut newer = back.clone();
        newer.new_stems();
        assert_eq!(
            (newer.stems_generation, newer.stem_overviews.clone()),
            (2, None)
        );
        let mut stale = newer.clone();
        stale.stem_overviews = back.stem_overviews.clone();
        assert!(
            !stale.stem_overviews_for(100),
            "the overviews of older stems"
        );
        let mut other = back.clone();
        other.stem_overviews.as_mut().unwrap().format = "f32le-2".into();
        assert!(!other.stem_overviews_for(100), "another format");
        // A `meta` of an older build has neither line.
        let old = Meta::parse("name\tacme\nrate\t44100\nstem\tbass\n");
        assert_eq!((old.stems_generation, old.stem_overviews), (0, None));
        assert_eq!(stem_overview_file(3), "overviews/stems/3.f32");
        assert_eq!(
            stem_overview_part(STEM_MIX_OVERVIEW, 2),
            "overviews/stems/mix.f32.2.decode.part"
        );
        assert_eq!(overview_bytes(4_097), 8 * 2_049);
    }

    /// A session of `sources` sources and `stems` stem lines.
    fn session_of(sources: usize, stems: usize) -> Meta {
        Meta {
            name: "lloyd".into(),
            rate: 44_100,
            files: (0..sources)
                .map(|k| (format!("acme {k}.wav"), format!("{k}.wav")))
                .collect(),
            stems: (0..stems).map(|k| format!("stem {k}")).collect(),
            ..Meta::default()
        }
    }

    /// D2 of the M7 plan: the lanes shown, with one 16-bit lane for
    /// classic drums, must fit the read-ahead buffer at its minimum slots.
    /// An f32 lane takes 256 KiB a slot and a 16-bit lane 128 KiB; at 4
    /// and 2 slots, 32 MiB holds about 5,461 KiB a slot.
    #[test]
    fn the_lanes_must_fit_the_read_ahead() {
        use engine::readahead::LaneFormat::{F32, Pcm16};
        // A separated song: six stems, the original and classic drums.
        let separated = session_of(1, 6);
        assert_eq!(
            open_formats(&separated),
            [Pcm16, Pcm16, Pcm16, Pcm16, Pcm16, Pcm16, F32, Pcm16]
        );
        assert!(fits_read_ahead(&open_formats(&separated)));
        // With stem lines, the sources are not shown.
        assert!(fits_read_ahead(&open_formats(&session_of(40, 6))));
        // With no stem lines, each source is an f32 lane: 20 fit, 21 not.
        let twenty = session_of(20, 0);
        assert_eq!(open_formats(&twenty).len(), 21);
        assert_eq!(open_formats(&twenty)[..20], [F32; 20]);
        assert_eq!(open_formats(&twenty)[20], Pcm16);
        assert!(fits_read_ahead(&open_formats(&twenty)));
        assert!(!fits_read_ahead(&open_formats(&session_of(21, 0))));
        // Files that are not sources are not lanes.
        let mut with_mix = session_of(20, 0);
        with_mix.files.push(("mix.wav".into(), "20.wav".into()));
        assert!(fits_read_ahead(&open_formats(&with_mix)));
        assert!(fits_read_ahead(&[]));
    }

    #[test]
    fn meta_round_trip() {
        let m = Meta {
            name: "song\tname".into(),
            rate: 44_100,
            files: vec![("a b.mp3".into(), "0.mp3".into())],
            stems: vec!["guitar".into(), "classic drums".into()],
            mix: None,
            decoded: None,
            ..Meta::default()
        };
        let back = Meta::parse(&m.to_text());
        assert_eq!(back.name, "song name");
        assert_eq!(back.files, m.files);
        assert_eq!(back.stems, m.stems);
        assert_eq!(back.rate, 44_100);
        assert_eq!(back.mix, None);
    }

    /// D1 of M4: the `mix` line reads back, and a mix of another format
    /// is not usable.
    #[test]
    fn the_mix_line_reads_back_with_its_format() {
        let m = Meta {
            name: "acme".into(),
            rate: 44_100,
            mix: Some((10_584_000, MIX_FORMAT.into())),
            ..Meta::default()
        };
        let text = m.to_text();
        assert!(text.contains("mix\t10584000\tf32le-1\n"), "{text}");
        let back = Meta::parse(&text);
        assert_eq!(back, m);
        assert_eq!(back.usable_mix(), Some(10_584_000));
        let other = Meta::parse("name\tacme\nmix\t4\tf32le-2\n");
        assert_eq!(other.mix, Some((4, "f32le-2".into())));
        assert_eq!(other.usable_mix(), None, "an unknown format tag");
        assert_eq!(Meta::parse("mix\tlots\tf32le-1\n").mix, None);
        assert_eq!(Meta::parse("name\tacme\n").usable_mix(), None);
    }

    /// D2 of M5: the `decoded` line reads back, with the channels of each
    /// source.
    #[test]
    fn the_decoded_line_reads_back_with_its_format() {
        let m = Meta {
            name: "acme".into(),
            rate: 44_100,
            files: vec![("drums.wav".into(), "0.wav".into())],
            decoded: Some(Decoded {
                frames: 10_584_000,
                format: MIX_FORMAT.into(),
                channels: vec![(0, 2), (2, 1)],
            }),
            ..Meta::default()
        };
        let text = m.to_text();
        assert!(
            text.contains("decoded\t10584000\tf32le-1\t0:2,2:1\n"),
            "{text}"
        );
        assert_eq!(Meta::parse(&text), m);
        for bad in [
            "decoded\tlots\tf32le-1\t0:2\n",
            "decoded\t4\tf32le-1\t0:3\n",
            "decoded\t4\tf32le-1\t0-2\n",
            "decoded\t4\tf32le-1\n",
        ] {
            assert_eq!(Meta::parse(bad).decoded, None, "{bad:?}");
        }
    }

    fn files(names: &[(&str, &str)]) -> Meta {
        Meta {
            name: "lloyd".into(),
            rate: 44_100,
            files: names
                .iter()
                .map(|(orig, stored)| (orig.to_string(), stored.to_string()))
                .collect(),
            ..Meta::default()
        }
    }

    fn source(index: u32, name: &str, ext: &str, in_mix: bool) -> PlannedSource {
        PlannedSource {
            index,
            name: name.into(),
            file: format!("src/{index}.{}", if ext == "flac" { "flac" } else { ext }),
            ext: ext.into(),
            in_mix,
        }
    }

    /// D6: the sources go in the order of `Song::from_audio`, with no file
    /// that is not a stem, and a derived source out of the mix.
    #[test]
    fn the_sources_are_planned_in_the_order_of_from_audio() {
        let meta = files(&[
            ("music/Vocals.wav", "0.wav"),
            ("mix.wav", "1.wav"),
            ("classic drums.wav", "2.wav"),
            ("drums.FLAC", "3.flac"),
            ("lloyd.mp3", "4.mp3"),
        ]);
        let sources = planned_sources(&meta).expect("planned");
        assert_eq!(
            sources,
            [
                source(3, "drums", "flac", true),
                source(0, "vocals", "wav", true),
                source(2, "classic drums", "wav", false),
                source(4, "lloyd", "mp3", true),
            ]
        );
        assert!(!one_source(&sources));
        let error = planned_sources(&files(&[("drums.wav", "drums.wav")]));
        assert!(error.is_err(), "{error:?}");
        assert_eq!(
            planned_sources(&files(&[("mix.wav", "0.wav")])),
            Ok(Vec::new())
        );
    }

    /// D1: a song of one source keeps that source in `mix.f32`, and a
    /// derived source in its own file.
    #[test]
    fn a_song_of_one_source_keeps_its_source_in_the_mix_file() {
        let meta = files(&[("drums.wav", "0.wav"), ("classic drums.wav", "1.wav")]);
        let sources = planned_sources(&meta).expect("planned");
        assert!(one_source(&sources));
        assert_eq!(source_file(&sources[0], true), "mix.f32");
        assert_eq!(source_file(&sources[1], true), "decoded/1.f32");
        assert_eq!(source_file(&sources[0], false), "decoded/0.f32");
        let frames = 10_000;
        let overview = 8 * engine::overview::column_count(frames) as u64;
        assert_eq!(
            session_files(frames, &sources),
            [
                ("mix.f32".to_string(), 80_000),
                ("decoded/1.f32".to_string(), 80_000),
                ("overviews/0.f32".to_string(), overview),
                ("overviews/1.f32".to_string(), overview),
                ("overviews/mix.f32".to_string(), overview),
            ]
        );
    }

    /// D2: a session is complete only with a `decoded` line of this format,
    /// at least one frame, the channels of each source, and every file of
    /// its size.
    #[test]
    fn a_decoded_line_of_another_format_or_a_size_that_differs_gives_an_incomplete_session() {
        let mut meta = files(&[("bass.wav", "0.wav"), ("drums.wav", "1.wav")]);
        let frames = 5_000;
        meta.decoded = Some(Decoded {
            frames,
            format: MIX_FORMAT.into(),
            channels: vec![(0, 2), (1, 1)],
        });
        let sources = planned_sources(&meta).expect("planned");
        let sizes: std::collections::HashMap<String, u64> =
            session_files(frames, &sources).into_iter().collect();
        assert_eq!(sizes.len(), 6, "{sizes:?}");
        let size_of = |path: &str| sizes.get(path).copied();
        assert_eq!(complete_frames(&meta, &sources, size_of), Some(frames));
        for path in sizes.keys() {
            for wrong in [None, Some(sizes[path] - 1), Some(sizes[path] + 8)] {
                let size_of = |p: &str| {
                    if p == path {
                        wrong
                    } else {
                        sizes.get(p).copied()
                    }
                };
                assert_eq!(
                    complete_frames(&meta, &sources, size_of),
                    None,
                    "{path}: {wrong:?}"
                );
            }
        }
        let with = |change: fn(&mut Decoded)| {
            let mut m = meta.clone();
            m.decoded.as_mut().map(change);
            m
        };
        for (case, m) in [
            ("another format", with(|d| d.format = "f32le-2".into())),
            ("no channels for a source", with(|d| d.channels.truncate(1))),
            ("no frames", with(|d| d.frames = 0)),
            (
                "no line",
                Meta {
                    decoded: None,
                    ..meta.clone()
                },
            ),
        ] {
            assert_eq!(complete_frames(&m, &sources, size_of), None, "{case}");
        }
    }
}

/// `"music/Queen - Song.mp3"` becomes `"Queen - Song"`: the name shown for
/// a stored song and used for downloaded stems.
pub fn file_title(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match base.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => base.to_string(),
    }
}

#[cfg(test)]
mod title_tests {
    #[test]
    fn titles() {
        assert_eq!(super::file_title("music/Queen - Song.mp3"), "Queen - Song");
        assert_eq!(super::file_title("a.b.wav"), "a.b");
        assert_eq!(super::file_title(".hidden"), ".hidden");
        assert_eq!(super::file_title("noext"), "noext");
    }
}

/// The session file that a stored lane plays from (D8 of
/// `plans/active/stream-from-storage-m6.md`): its path from the root of
/// browser storage, its format, and where its frames are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneFile {
    pub path: String,
    pub format: engine::readahead::LaneFormat,
    /// The bytes before the first frame.
    pub data_offset: u64,
    /// The frames that the file holds.
    pub data_frames: u64,
}

impl LaneFile {
    /// An f32 session file of `frames` frames, such as `mix.f32`.
    pub fn f32(path: String, frames: usize) -> Self {
        Self {
            path,
            format: engine::readahead::LaneFormat::F32,
            data_offset: 0,
            data_frames: frames as u64,
        }
    }

    /// The byte range `(start, end)` of up to `frames` frames from frame
    /// `first`, within the file's data: empty past its end.
    pub fn byte_range(&self, first: u64, frames: u64) -> (u64, u64) {
        let per_frame = self.format.bytes_per_frame() as u64;
        let start = first.min(self.data_frames);
        let end = (first + frames).min(self.data_frames);
        (
            self.data_offset + start * per_frame,
            self.data_offset + end * per_frame,
        )
    }
}

/// The lane file of each source of a complete session in the folder
/// `folder`, in the order of `sources`, for a song of `frames` (D8).
pub fn source_lane_files(folder: &str, sources: &[PlannedSource], frames: usize) -> Vec<LaneFile> {
    let one = one_source(sources);
    sources
        .iter()
        .map(|source| LaneFile::f32(format!("{folder}/{}", source_file(source, one)), frames))
        .collect()
}

/// The lane file of the mix of a session in `folder` whose usable mix has
/// `frames` frames: for the "original" lane of a song of several sources.
pub fn mix_lane_file(folder: &str, frames: usize) -> LaneFile {
    LaneFile::f32(format!("{folder}/mix.f32"), frames)
}

/// Where the samples of a WAV file are, from its first bytes `head`: the
/// offset of its data and its frames.  Only 16-bit stereo PCM at 44.1 kHz
/// is taken; any other format, or a header past `head`, gives `None`.
pub fn wav_data(head: &[u8]) -> Option<(u64, u64)> {
    let u16_at = |at: usize| Some(u16::from_le_bytes([*head.get(at)?, *head.get(at + 1)?]));
    let u32_at = |at: usize| {
        Some(u32::from_le_bytes([
            *head.get(at)?,
            *head.get(at + 1)?,
            *head.get(at + 2)?,
            *head.get(at + 3)?,
        ]))
    };
    if head.get(..4)? != b"RIFF" || head.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let mut pcm16 = false;
    loop {
        let id = head.get(at..at + 4)?;
        let size = u32_at(at + 4)? as usize;
        let body = at + 8;
        if id == b"fmt " {
            pcm16 = u16_at(body)? == 1
                && u16_at(body + 2)? == 2
                && u32_at(body + 4)? == 44_100
                && u16_at(body + 12)? == 4
                && u16_at(body + 14)? == 16;
        } else if id == b"data" {
            return pcm16.then_some((body as u64, size as u64 / 4));
        }
        // Chunks are padded to an even size.
        at = body.checked_add(size)?.checked_add(size % 2)?;
    }
}

/// The lane file of the stem WAV at `path`, whose first bytes are `head`
/// and whose size is `file_bytes`, for a song of `frames` (D8).  It gives
/// one only if the file is complete: it holds at least the song's frames
/// less one.  A `.part` file, which the next separation writes, gives none.
pub fn stem_lane_file(path: &str, head: &[u8], file_bytes: u64, frames: usize) -> Option<LaneFile> {
    if path.ends_with(".part") {
        return None;
    }
    let (data_offset, header_frames) = wav_data(head)?;
    // A file cut short holds only the frames that it has.
    let data_frames = header_frames.min(file_bytes.saturating_sub(data_offset) / 4);
    (data_frames + 1 >= frames as u64).then(|| LaneFile {
        path: path.to_string(),
        format: engine::readahead::LaneFormat::Pcm16,
        data_offset,
        data_frames,
    })
}

#[cfg(test)]
mod lane_tests {
    use super::*;
    use engine::readahead::LaneFormat;

    fn source(index: u32, name: &str, in_mix: bool) -> PlannedSource {
        PlannedSource {
            index,
            name: name.into(),
            file: format!("src/{index}.wav"),
            ext: "wav".into(),
            in_mix,
        }
    }

    #[test]
    fn the_lane_files_of_a_session() {
        let folder = "sessions/42";
        // One source: its lane plays from `mix.f32`.
        let one = [source(0, "acme", true)];
        assert_eq!(
            source_lane_files(folder, &one, 1_000),
            [LaneFile::f32("sessions/42/mix.f32".into(), 1_000)]
        );
        // Several sources, with a derived source: each from its own file.
        let several = [
            source(2, "drums", true),
            source(0, "bass", true),
            source(1, "classic drums", false),
        ];
        assert_eq!(
            source_lane_files(folder, &several, 500),
            [
                LaneFile::f32("sessions/42/decoded/2.f32".into(), 500),
                LaneFile::f32("sessions/42/decoded/0.f32".into(), 500),
                LaneFile::f32("sessions/42/decoded/1.f32".into(), 500),
            ]
        );
        // One source in the mix and a derived source: the one in the mix
        // plays from `mix.f32`.
        let derived = [source(0, "vocals", true), source(1, "classic drums", false)];
        assert_eq!(
            source_lane_files(folder, &derived, 7),
            [
                LaneFile::f32("sessions/42/mix.f32".into(), 7),
                LaneFile::f32("sessions/42/decoded/1.f32".into(), 7),
            ]
        );
        // The "original" of several sources plays from `mix.f32`.
        assert_eq!(
            mix_lane_file(folder, 500),
            LaneFile::f32("sessions/42/mix.f32".into(), 500)
        );
        // A stem from its WAV.
        let head = engine::wav::header(1_000, 44_100);
        let path = "sessions/42/stems/vocals.wav".to_string();
        assert_eq!(
            stem_lane_file(&path, &head, 44 + 4 * 1_000, 1_000),
            Some(LaneFile {
                path: path.clone(),
                format: LaneFormat::Pcm16,
                data_offset: 44,
                data_frames: 1_000,
            })
        );
        // One frame short is complete; two are not.
        assert!(
            stem_lane_file(
                &path,
                &engine::wav::header(999, 44_100),
                44 + 4 * 999,
                1_000
            )
            .is_some()
        );
        assert!(
            stem_lane_file(
                &path,
                &engine::wav::header(998, 44_100),
                44 + 4 * 998,
                1_000
            )
            .is_none()
        );
        // A `.part` file is never a lane file.
        assert!(stem_lane_file(&format!("{path}.part"), &head, 44 + 4 * 1_000, 1_000).is_none());
    }

    #[test]
    fn a_lane_file_reads_within_its_data() {
        let wav = LaneFile {
            path: "sessions/42/stems/bass.wav".into(),
            format: LaneFormat::Pcm16,
            data_offset: 44,
            data_frames: 100,
        };
        assert_eq!(wav.byte_range(0, 32), (44, 44 + 128));
        assert_eq!(
            wav.byte_range(90, 32),
            (44 + 360, 44 + 400),
            "cut at its end"
        );
        assert_eq!(
            wav.byte_range(100, 32),
            (44 + 400, 44 + 400),
            "past its end"
        );
        assert_eq!(wav.byte_range(500, 32), (44 + 400, 44 + 400));
        let mix = LaneFile::f32("sessions/42/mix.f32".into(), 10);
        assert_eq!(mix.byte_range(4, 4), (32, 64));
    }

    /// A WAV header with a `fmt ` chunk of the given fields, an extra
    /// chunk before `data`, and `frames` frames of data.
    fn wav(format: u16, channels: u16, rate: u32, bits: u16, frames: u32) -> Vec<u8> {
        let block = channels * bits / 8;
        let mut out = Vec::new();
        out.extend(b"RIFF");
        out.extend(0_u32.to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16_u32.to_le_bytes());
        out.extend(format.to_le_bytes());
        out.extend(channels.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * u32::from(block)).to_le_bytes());
        out.extend(block.to_le_bytes());
        out.extend(bits.to_le_bytes());
        out.extend(b"LIST");
        out.extend(3_u32.to_le_bytes());
        out.extend(b"abc\0");
        out.extend(b"data");
        out.extend((frames * u32::from(block)).to_le_bytes());
        out
    }

    /// Review round 1 of M6, finding 1-11: a WAV whose file ends before
    /// its header says holds only the frames in the file.
    #[test]
    fn a_cut_wav_holds_only_the_frames_in_its_file() {
        let path = "sessions/1/stems/bass.wav";
        let head = engine::wav::header(1_000, 44_100);
        let whole = stem_lane_file(path, &head, 44 + 4 * 1_000, 1_000);
        assert_eq!(whole.map(|f| f.data_frames), Some(1_000));
        let short = stem_lane_file(path, &head, 44 + 4 * 999, 1_000);
        assert_eq!(short.map(|f| f.data_frames), Some(999), "one frame short");
        assert_eq!(
            stem_lane_file(path, &head, 44 + 4 * 500, 1_000),
            None,
            "cut"
        );
    }

    #[test]
    fn a_wav_header_gives_its_data_and_frames() {
        // The header that `engine::wav` writes.
        assert_eq!(
            wav_data(&engine::wav::header(1_234, 44_100)),
            Some((44, 1_234))
        );
        // A chunk before `data`, padded to an even size, is skipped.
        assert_eq!(wav_data(&wav(1, 2, 44_100, 16, 77)), Some((56, 77)));
        // Other formats are refused.
        for other in [
            wav(3, 2, 44_100, 32, 10),
            wav(1, 1, 44_100, 16, 10),
            wav(1, 2, 48_000, 16, 10),
            wav(1, 2, 44_100, 24, 10),
        ] {
            assert_eq!(wav_data(&other), None);
        }
        // Mutants 1-21 and 1-33 of review round 1 of M6: an RF64 header,
        // and a field whose high byte is not 0.
        let mut rf64 = engine::wav::header(10, 44_100);
        rf64[..4].copy_from_slice(b"RF64");
        assert_eq!(wav_data(&rf64), None);
        let mut bits = engine::wav::header(10, 44_100);
        bits[35] = 1;
        assert_eq!(wav_data(&bits), None, "bits per sample 0x0110");
        // A header cut short, or no RIFF at all.
        let full = engine::wav::header(10, 44_100);
        assert_eq!(wav_data(&full[..40]), None);
        assert_eq!(
            wav_data(b"not a wav file at all, but long enough to read"),
            None
        );
        // A stem one frame short of the song is a lane file; a 48 kHz one
        // is not.
        let path = "sessions/1/stems/bass.wav";
        assert!(stem_lane_file(path, &engine::wav::header(99, 44_100), 44 + 4 * 99, 100).is_some());
        assert!(
            stem_lane_file(path, &engine::wav::header(100, 48_000), 44 + 4 * 100, 100).is_none()
        );
    }
}
