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
