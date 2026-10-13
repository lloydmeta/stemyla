//! The `meta` file of a stored song session (see `store`): plain
//! tab-separated lines, readable without the browser.

/// What a session holds, from its `meta` file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub name: String,
    pub rate: u32,
    /// (original file name, stored file name)
    pub files: Vec<(String, String)>,
    pub stems: Vec<String>,
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
        s
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
        };
        let back = Meta::parse(&m.to_text());
        assert_eq!(back.name, "song name");
        assert_eq!(back.files, m.files);
        assert_eq!(back.stems, m.stems);
        assert_eq!(back.rate, 44_100);
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
