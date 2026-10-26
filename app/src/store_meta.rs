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
        s
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_round_trip() {
        let m = Meta {
            name: "song\tname".into(),
            rate: 44_100,
            files: vec![("a b.mp3".into(), "0.mp3".into())],
            stems: vec!["guitar".into(), "classic drums".into()],
            mix: None,
            decoded: None,
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
