//! Which browser the browser tests drive, and where its WebDriver is.
//!
//! * Safari: `/usr/bin/safaridriver`, which ships with macOS; turn it on
//!   once with `safaridriver --enable`.
//! * Chrome: `$CHROMEDRIVER`, else `$CHROMEWEBDRIVER/chromedriver` (set on
//!   GitHub's runners), else `chromedriver` on `PATH`.  It must match the
//!   installed Chrome's major version.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Safari,
}

impl Browser {
    /// Safari on macOS (no download, and the engine iOS uses), Chrome
    /// elsewhere.
    pub fn default_here() -> Self {
        if cfg!(target_os = "macos") {
            Self::Safari
        } else {
            Self::Chrome
        }
    }

    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "chrome" => Ok(Self::Chrome),
            "safari" => Ok(Self::Safari),
            other => bail!("unknown browser {other} (chrome or safari)"),
        }
    }

    /// The WebDriver executable for this browser.
    pub fn driver(self) -> Result<PathBuf> {
        match self {
            Self::Safari => {
                let path = PathBuf::from("/usr/bin/safaridriver");
                if !path.exists() {
                    bail!(
                        "no {} (Safari's WebDriver ships with macOS)",
                        path.display()
                    );
                }
                Ok(path)
            }
            Self::Chrome => chromedriver(
                std::env::var_os("CHROMEDRIVER").map(PathBuf::from),
                std::env::var_os("CHROMEWEBDRIVER").map(PathBuf::from),
                std::env::var_os("PATH").unwrap_or_default(),
            ),
        }
    }

    /// What to do when the driver will not start a session.
    pub fn setup_hint(self) -> &'static str {
        match self {
            Self::Safari => {
                "Safari: run `safaridriver --enable` once, and tick Develop > Allow Remote Automation in Safari"
            }
            Self::Chrome => {
                "Chrome: chromedriver must match Chrome's major version (Chrome for Testing: https://googlechromelabs.github.io/chrome-for-testing/); point CHROMEDRIVER at it"
            }
        }
    }
}

fn chromedriver(
    explicit: Option<PathBuf>,
    ci_dir: Option<PathBuf>,
    path: std::ffi::OsString,
) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if !p.is_file() {
            bail!("CHROMEDRIVER is {}, which is not a file", p.display());
        }
        return Ok(p);
    }
    let name = Path::new("chromedriver").with_extension(std::env::consts::EXE_EXTENSION);
    let candidates = ci_dir
        .into_iter()
        .chain(std::env::split_paths(&path))
        .map(|d| d.join(&name));
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    bail!(
        "no chromedriver: set CHROMEDRIVER to one matching your Chrome (https://googlechromelabs.github.io/chrome-for-testing/)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xtask-drivers-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_browser_names() {
        assert_eq!(Browser::parse("chrome").unwrap(), Browser::Chrome);
        assert_eq!(Browser::parse("safari").unwrap(), Browser::Safari);
        assert!(Browser::parse("firefox").is_err());
    }

    #[test]
    fn an_explicit_chromedriver_wins_and_must_exist() {
        let d = temp_dir("explicit");
        let f = d.join("my-chromedriver");
        std::fs::write(&f, "").unwrap();
        assert_eq!(chromedriver(Some(f.clone()), None, "".into()).unwrap(), f);
        assert!(chromedriver(Some(d.join("missing")), None, "".into()).is_err());
    }

    #[test]
    fn the_ci_directory_comes_before_path() {
        let (ci, on_path) = (temp_dir("ci"), temp_dir("path"));
        let name = Path::new("chromedriver").with_extension(std::env::consts::EXE_EXTENSION);
        std::fs::write(on_path.join(&name), "").unwrap();
        let path = std::env::join_paths([&on_path]).unwrap();
        assert_eq!(
            chromedriver(None, Some(ci.clone()), path.clone()).unwrap(),
            on_path.join(&name),
            "an empty CI directory is skipped"
        );
        std::fs::write(ci.join(&name), "").unwrap();
        assert_eq!(
            chromedriver(None, Some(ci.clone()), path).unwrap(),
            ci.join(&name)
        );
        assert!(chromedriver(None, None, "".into()).is_err());
    }
}
