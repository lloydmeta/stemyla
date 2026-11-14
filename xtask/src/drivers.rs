//! Which browser the browser tests drive, and where its WebDriver is.
//!
//! * Safari: `/usr/bin/safaridriver`, which ships with macOS; turn it on
//!   once with `safaridriver --enable`.
//! * Chrome: `$CHROMEDRIVER`, else `$CHROMEWEBDRIVER/chromedriver` (set on
//!   GitHub's runners), else `chromedriver` on `PATH`.  It must match the
//!   installed Chrome's major version.
//!
//! A Safari run on macOS checks the screen lock first, and holds
//! `caffeinate -u` while it runs.  It ends each stale `safaridriver`
//! before each `e2e` session start, and before each `web-test` crate run
//! (TD-96).  It lists Safari's folder in the user's temporary folder at
//! its start and at its end, and prints what the run left there (TD-103).
//! It removes nothing there.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

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

/// The key of a console session in the plist of `ioreg -n Root -d1 -a`.
/// macOS leaves it out while the screen is unlocked.
const LOCK_KEY: &str = "<key>CGSSessionScreenIsLocked</key>";

/// What `web-test` and `e2e` give when they refuse a Safari run.
const LOCKED_TEXT: &str = "the screen is locked, and safaridriver cannot start a Safari session then: it waits, and gives \"The session timed out while connecting to a Safari instance\".  Unlock the screen, and run again (`ioreg -n Root -d1 -a | grep -A1 CGSSessionScreenIsLocked` gives nothing when it is unlocked)";

/// Whether the plist of `ioreg -n Root -d1 -a` shows a locked screen: a
/// session has the lock key, with the value `<true/>`.
pub fn screen_is_locked(plist: &str) -> bool {
    plist
        .match_indices(LOCK_KEY)
        .any(|(at, key)| plist[at + key.len()..].trim_start().starts_with("<true/>"))
}

/// Refuses a Safari run on macOS while the screen is locked, before a
/// driver starts.  `on_macos` tells whether this is macOS, and `ioreg`
/// gives the plist of [`ioreg_root`].  With no plist, it says so, and
/// goes on.
pub fn refuse_a_locked_screen(
    browser: Browser,
    on_macos: bool,
    ioreg: impl FnOnce() -> Option<String>,
) -> Result<()> {
    if browser != Browser::Safari || !on_macos {
        return Ok(());
    }
    match ioreg() {
        Some(plist) if screen_is_locked(&plist) => bail!("{LOCKED_TEXT}"),
        Some(_) => {}
        None => eprintln!("the screen lock is not known: /usr/sbin/ioreg gave no output"),
    }
    Ok(())
}

/// The plist of `ioreg -n Root -d1 -a`, or `None` when `ioreg` fails.
pub fn ioreg_root() -> Option<String> {
    crate::stdout_on_success(Command::new("/usr/sbin/ioreg").args(["-n", "Root", "-d1", "-a"]))
}

/// The longest that [`keep_user_active`] holds its assertion: longer than
/// any `web-test` or `e2e` run (`e2e save` may take 60 minutes in CI).
const ACTIVE_LIMIT_S: u64 = 4 * 60 * 60;

/// Holds the assertion that the user is active, and ends it when dropped.
pub struct UserActive(Option<Child>);

impl Drop for UserActive {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The arguments of `caffeinate` that declare the user active for `secs`,
/// or until the process `pid` ends.  With no `-t`, `-u` holds only 5 s.
fn user_active_args(pid: u32, secs: u64) -> [String; 5] {
    [
        "-u".to_string(),
        "-t".to_string(),
        secs.to_string(),
        "-w".to_string(),
        pid.to_string(),
    ]
}

/// Safari keeps each `AudioContext` suspended while macOS reports the user
/// idle (`references/browser-quirks.md`).  So a Safari run on macOS
/// declares the user active with `caffeinate` ([`caffeinate`]) while it
/// runs.  `on_macos` tells whether this is macOS.
pub fn keep_user_active(
    browser: Browser,
    on_macos: bool,
    caffeinate: impl FnOnce() -> std::io::Result<Child>,
) -> UserActive {
    if browser != Browser::Safari || !on_macos {
        return UserActive(None);
    }
    let child = caffeinate();
    if let Err(e) = &child {
        eprintln!("caffeinate did not start ({e}), so Safari may keep its audio suspended");
    }
    UserActive(child.ok())
}

/// Starts `caffeinate -u`, which declares the user active for
/// [`ACTIVE_LIMIT_S`], or until this process ends.
pub fn caffeinate() -> std::io::Result<Child> {
    Command::new("/usr/bin/caffeinate")
        .args(user_active_args(std::process::id(), ACTIVE_LIMIT_S))
        .spawn()
}

/// The pids of the `safaridriver`s in `ps -x -o pid=,ppid=,comm=` whose
/// parent is gone (`launchd`, pid 1).  A runner or an `e2e` that was
/// stopped leaves such a driver, and its session stays paired with Safari.
pub fn stale_safaridrivers(ps: &str) -> Vec<u32> {
    ps.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?;
            let command = fields.next()?;
            (parent == "1" && command == "/usr/bin/safaridriver").then_some(pid)
        })
        .collect()
}

/// The processes of this user, as `ps -x -o pid=,ppid=,comm=` lists them,
/// or `None` when `ps` fails.
pub fn processes() -> Option<String> {
    crate::stdout_on_success(Command::new("/bin/ps").args(["-x", "-o", "pid=,ppid=,comm="]))
}

/// The process ID and the whole command line of each process of each user,
/// one line each, as `ps -A -ww -o pid=,command=` lists them, or `None`
/// when `ps` fails.
pub fn command_lines() -> Option<String> {
    crate::stdout_on_success(Command::new("/bin/ps").args(["-A", "-ww", "-o", "pid=,command="]))
}

/// Ends each stale `safaridriver` in `ps` (the text of [`processes`]), so
/// a new session can pair with Safari.  It gives `run` one `kill` command
/// for each stale driver, and ends no driver that a running program owns.
pub fn end_stale_safari_sessions(ps: Option<&str>, mut run: impl FnMut(Command)) {
    for pid in stale_safaridrivers(ps.unwrap_or_default()) {
        eprintln!("ending the stale safaridriver {pid}, whose parent is gone");
        let mut kill = Command::new("/bin/kill");
        kill.arg(pid.to_string());
        run(kill);
    }
}

/// The folder of Safari's network process in the user's temporary folder
/// (`getconf DARWIN_USER_TEMP_DIR`).  WebKit keeps a write stream's swap
/// file (`FileSystemWritableStream*`) and each blob folder
/// (`BlobRegistryFiles-*`) there.
const SAFARI_FOLDER: &str = "com.apple.WebKit.Networking+com.apple.Safari";

/// The start of a swap file's name.
const SWAP_FILE: &str = "FileSystemWritableStream";

/// The start of a blob folder's name.
const BLOB_FOLDER: &str = "BlobRegistryFiles-";

/// The kind of an entry of Safari's folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
    /// A symbolic link or another kind, which the report does not count.
    Other,
}

/// One entry of Safari's folder: its name, its kind, and its bytes.  The
/// bytes of a folder are the bytes of the files in it, at any depth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub bytes: u64,
}

/// What a run left in Safari's folder: the new swap files and blob
/// folders, with their bytes, and the growth of the old ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Left {
    pub swap_files: u64,
    pub swap_bytes: u64,
    pub blob_folders: u64,
    pub blob_bytes: u64,
    /// The bytes that the swap files and blob folders of `before` grew by.
    pub grown_bytes: u64,
}

/// What is new in the listing `after` against the listing `before`.  An
/// entry that `after` does not have counts for nothing.
pub fn left_behind(before: &[Entry], after: &[Entry]) -> Left {
    let mut left = Left::default();
    for entry in after {
        let swap = entry.kind == EntryKind::File && entry.name.starts_with(SWAP_FILE);
        let blob = entry.kind == EntryKind::Folder && entry.name.starts_with(BLOB_FOLDER);
        if !swap && !blob {
            continue;
        }
        let old = before
            .iter()
            .find(|old| old.name == entry.name && old.kind == entry.kind);
        match (old, swap) {
            (Some(old), _) => left.grown_bytes += entry.bytes.saturating_sub(old.bytes),
            (None, true) => {
                left.swap_files += 1;
                left.swap_bytes += entry.bytes;
            }
            (None, false) => {
                left.blob_folders += 1;
                left.blob_bytes += entry.bytes;
            }
        }
    }
    left
}

/// `count` with its noun, in the singular for 1.
fn counted(count: u64, noun: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{plural}")
}

/// `bytes` in megabytes (10^6 bytes), with 1 decimal.
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

/// The line that a Safari run prints about what it left in `folder`.
pub fn left_line(left: &Left, folder: &Path) -> String {
    let grown = if left.grown_bytes > 0 {
        format!(
            ", and its older ones grew by {}",
            megabytes(left.grown_bytes)
        )
    } else {
        String::new()
    };
    format!(
        "browser data: Safari left {} ({}) and {} ({}) in {}{grown}; xtask does not remove them (TD-103)",
        counted(left.swap_files, "swap file"),
        megabytes(left.swap_bytes),
        counted(left.blob_folders, "blob folder"),
        megabytes(left.blob_bytes),
        folder.display()
    )
}

/// The entries of `folder`, and no others.  A folder that is not there
/// gives none.  It follows no symbolic link.
pub fn list_folder(folder: &Path) -> Vec<Entry> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            // `DirEntry::metadata` does not follow a symbolic link.
            let meta = entry.metadata().ok()?;
            let (kind, bytes) = if meta.is_file() {
                (EntryKind::File, meta.len())
            } else if meta.is_dir() {
                (EntryKind::Folder, bytes_in(&entry.path()))
            } else {
                (EntryKind::Other, 0)
            };
            Some(Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                kind,
                bytes,
            })
        })
        .collect()
}

/// The bytes of the files in `folder`, at any depth.  It follows no
/// symbolic link.
fn bytes_in(folder: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some(if meta.is_file() {
                meta.len()
            } else if meta.is_dir() {
                bytes_in(&entry.path())
            } else {
                0
            })
        })
        .sum()
}

/// The listing of Safari's folder at the start of a Safari run on macOS.
/// A drop lists the folder again, and prints [`left_line`].  It does not
/// fail the run: the person's own Safari can add files in the same time.
/// It removes nothing.
pub struct SafariData {
    folder: PathBuf,
    before: Vec<Entry>,
    /// Where the line goes: stdout, or a test's record.
    print: fn(&str),
}

impl Drop for SafariData {
    fn drop(&mut self) {
        let left = left_behind(&self.before, &list_folder(&self.folder));
        (self.print)(&left_line(&left, &self.folder));
    }
}

fn print_line(line: &str) {
    println!("{line}");
}

/// Lists Safari's folder for a Safari run on macOS, and gives the
/// [`SafariData`] that reports at its drop.  `on_macos` tells whether this
/// is macOS, and `temp` gives the user's temporary folder.
pub fn watch_safari_data(
    browser: Browser,
    on_macos: bool,
    temp: impl FnOnce() -> Option<PathBuf>,
) -> Option<SafariData> {
    if browser != Browser::Safari || !on_macos {
        return None;
    }
    let Some(temp) = temp() else {
        eprintln!("browser data: no DARWIN_USER_TEMP_DIR, so no count of what Safari leaves");
        return None;
    };
    let folder = temp.join(SAFARI_FOLDER);
    Some(SafariData {
        before: list_folder(&folder),
        folder,
        print: print_line,
    })
}

/// The user's temporary folder, from `getconf DARWIN_USER_TEMP_DIR`.
pub fn darwin_user_temp_dir() -> Option<PathBuf> {
    let text =
        crate::stdout_on_success(Command::new("/usr/bin/getconf").arg("DARWIN_USER_TEMP_DIR"))?;
    let text = text.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
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
        // Empty, whatever an earlier run with the same process id left.
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The plist of `ioreg -n Root -d1 -a` around one console session.
    fn console_session(lock: &str) -> String {
        format!(
            "<dict>\n\t<key>IOConsoleUsers</key>\n\t<array>\n\t\t<dict>\n\t\t\t<key>CGSSessionUniqueSessionUUID</key>\n\t\t\t<string>E80B4492-3CD3-41D8-A9D9-30110D947097</string>\n{lock}\t\t\t<key>kCGSSessionOnConsoleKey</key>\n\t\t\t<true/>\n\t\t\t<key>kCGSSessionUserNameKey</key>\n\t\t\t<string>lloyd</string>\n\t\t</dict>\n\t</array>\n</dict>\n"
        )
    }

    #[test]
    fn a_session_with_the_lock_key_set_true_is_locked() {
        let plist = console_session("\t\t\t<key>CGSSessionScreenIsLocked</key>\n\t\t\t<true/>\n");
        assert!(screen_is_locked(&plist));
    }

    #[test]
    fn no_lock_key_or_a_false_one_is_unlocked() {
        assert!(!screen_is_locked(&console_session("")));
        let plist = console_session("\t\t\t<key>CGSSessionScreenIsLocked</key>\n\t\t\t<false/>\n");
        assert!(!screen_is_locked(&plist));
        assert!(!screen_is_locked(""));
    }

    const LOCKED: &str = "\t\t\t<key>CGSSessionScreenIsLocked</key>\n\t\t\t<true/>\n";

    #[test]
    fn safari_on_macos_refuses_a_locked_screen() {
        let error = refuse_a_locked_screen(Browser::Safari, true, || Some(console_session(LOCKED)))
            .unwrap_err();
        assert!(
            error.to_string().contains("the screen is locked"),
            "{error}"
        );
        assert!(
            refuse_a_locked_screen(Browser::Safari, true, || Some(console_session(""))).is_ok()
        );
    }

    #[test]
    fn safari_on_macos_goes_on_when_the_lock_is_not_known() {
        assert!(refuse_a_locked_screen(Browser::Safari, true, || None).is_ok());
    }

    #[test]
    fn only_safari_on_macos_reads_the_screen_lock() {
        let not_read = || -> Option<String> { panic!("ioreg was read") };
        assert!(refuse_a_locked_screen(Browser::Chrome, true, not_read).is_ok());
        assert!(refuse_a_locked_screen(Browser::Safari, false, not_read).is_ok());
        assert!(refuse_a_locked_screen(Browser::Chrome, false, not_read).is_ok());
    }

    #[test]
    fn ioreg_gives_the_root_plist_on_macos_only() {
        let plist = ioreg_root();
        if cfg!(target_os = "macos") {
            let plist = plist.expect("ioreg gives the root plist on macOS");
            assert!(plist.contains("<key>IOConsoleUsers</key>"), "{plist}");
        } else {
            assert_eq!(plist, None);
        }
    }

    #[test]
    fn the_user_stays_active_for_4_hours() {
        assert_eq!(ACTIVE_LIMIT_S, 14_400);
    }

    #[test]
    fn only_safari_on_macos_declares_the_user_active() {
        let not_started = || -> std::io::Result<Child> { panic!("caffeinate started") };
        assert!(
            keep_user_active(Browser::Chrome, true, not_started)
                .0
                .is_none()
        );
        assert!(
            keep_user_active(Browser::Safari, false, not_started)
                .0
                .is_none()
        );
        assert!(
            keep_user_active(Browser::Chrome, false, not_started)
                .0
                .is_none()
        );
        let started = std::cell::Cell::new(false);
        let held = keep_user_active(Browser::Safari, true, || {
            started.set(true);
            Err(std::io::Error::other("no caffeinate for lloyd"))
        });
        assert!(started.get(), "Safari on macOS starts caffeinate");
        assert!(
            held.0.is_none(),
            "a caffeinate that did not start holds nothing"
        );
    }

    #[test]
    fn each_stale_safaridriver_gets_one_kill() {
        let ps = "50712 50699 /usr/bin/safaridriver\n\
                  50801     1 /usr/bin/safaridriver\n\
                  50803     1 /usr/bin/safaridriver\n";
        let mut kills = Vec::new();
        end_stale_safari_sessions(Some(ps), |kill| {
            let args: Vec<String> = kill
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            kills.push((kill.get_program().to_string_lossy().into_owned(), args));
        });
        assert_eq!(
            kills,
            [
                ("/bin/kill".to_string(), vec!["50801".to_string()]),
                ("/bin/kill".to_string(), vec!["50803".to_string()]),
            ]
        );
    }

    #[test]
    fn no_process_list_ends_nothing() {
        end_stale_safari_sessions(None, |kill| panic!("{kill:?} ran"));
    }

    #[test]
    fn the_process_list_holds_this_process() {
        let ps = processes().expect("ps lists the processes of this user");
        let this = std::process::id().to_string();
        assert!(
            ps.lines()
                .any(|line| line.split_whitespace().next() == Some(this.as_str())),
            "no line for {this} in:\n{ps}"
        );
    }

    #[test]
    fn the_user_stays_active_for_the_limit_or_until_the_run_ends() {
        assert_eq!(
            user_active_args(4242, 14_400),
            ["-u", "-t", "14400", "-w", "4242"]
        );
    }

    #[test]
    fn only_a_safaridriver_with_no_parent_is_stale() {
        let ps = "  905     1 /usr/sbin/distnoted\n\
                  50712 50699 /usr/bin/safaridriver\n\
                  50713     1 /System/Library/PrivateFrameworks/WebDriver.framework/Versions/A/XPCServices/com.apple.WebDriver.HTTPService.xpc/Contents/MacOS/com.apple.WebDriver.HTTPService\n\
                  50801     1 /usr/bin/safaridriver\n\
                  50802     1 /Users/lloyd/bin/not-safaridriver\n";
        assert_eq!(stale_safaridrivers(ps), [50801]);
        assert_eq!(stale_safaridrivers(""), Vec::<u32>::new());
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

    fn entry(name: &str, kind: EntryKind, bytes: u64) -> Entry {
        Entry {
            name: name.to_string(),
            kind,
            bytes,
        }
    }

    /// Asserts:
    /// * a new swap file and a new blob folder count, with their bytes;
    /// * a removed swap file counts for nothing;
    /// * a swap file and a blob folder that grew give their growth;
    /// * an entry of another name, a link, and a folder named as a swap
    ///   file count for nothing.
    #[test]
    fn a_run_left_what_is_new_in_the_listing() {
        let before = [
            entry("FileSystemWritableStream.lloyd", EntryKind::File, 100),
            entry("FileSystemWritableStream.gone", EntryKind::File, 900),
            entry("BlobRegistryFiles-old", EntryKind::Folder, 10),
            entry("acme", EntryKind::File, 5),
        ];
        let after = [
            entry("FileSystemWritableStream.lloyd", EntryKind::File, 150),
            entry(
                "FileSystemWritableStream.new1",
                EntryKind::File,
                349_521_256,
            ),
            entry("FileSystemWritableStream.new2", EntryKind::File, 0),
            entry("BlobRegistryFiles-old", EntryKind::Folder, 30),
            entry("BlobRegistryFiles-new", EntryKind::Folder, 1_000_000),
            entry("acme", EntryKind::File, 500),
            entry("beachape", EntryKind::Folder, 500),
            entry("FileSystemWritableStream.link", EntryKind::Other, 7),
            entry("FileSystemWritableStream.folder", EntryKind::Folder, 7),
            entry("BlobRegistryFiles-file", EntryKind::File, 7),
        ];
        assert_eq!(
            left_behind(&before, &after),
            Left {
                swap_files: 2,
                swap_bytes: 349_521_256,
                blob_folders: 1,
                blob_bytes: 1_000_000,
                grown_bytes: 70,
            }
        );
        assert_eq!(left_behind(&after, &after), Left::default());
        assert_eq!(left_behind(&after, &[]), Left::default());
    }

    #[test]
    fn the_line_gives_the_counts_and_the_megabytes() {
        let folder = Path::new("/var/folders/lloyd/T/com.apple.WebKit.Networking+com.apple.Safari");
        let left = Left {
            swap_files: 12,
            swap_bytes: 350_070_000,
            blob_folders: 1,
            blob_bytes: 20_000,
            grown_bytes: 0,
        };
        assert_eq!(
            left_line(&left, folder),
            "browser data: Safari left 12 swap files (350.1 MB) and 1 blob folder (0.0 MB) in /var/folders/lloyd/T/com.apple.WebKit.Networking+com.apple.Safari; xtask does not remove them (TD-103)"
        );
        let one = Left {
            swap_files: 1,
            swap_bytes: 349_521_256,
            blob_folders: 2,
            blob_bytes: 0,
            grown_bytes: 1_260_000,
        };
        assert_eq!(
            left_line(&one, folder),
            "browser data: Safari left 1 swap file (349.5 MB) and 2 blob folders (0.0 MB) in /var/folders/lloyd/T/com.apple.WebKit.Networking+com.apple.Safari, and its older ones grew by 1.3 MB; xtask does not remove them (TD-103)"
        );
        assert_eq!(
            left_line(&Left::default(), folder),
            "browser data: Safari left 0 swap files (0.0 MB) and 0 blob folders (0.0 MB) in /var/folders/lloyd/T/com.apple.WebKit.Networking+com.apple.Safari; xtask does not remove them (TD-103)"
        );
    }

    /// Asserts:
    /// * the listing gives each file with its size, and each folder with
    ///   the bytes of the files in it, at any depth;
    /// * a symbolic link is `Other`, and its target is not read;
    /// * a folder that is not there gives no entry.
    #[test]
    fn the_listing_reads_the_named_folder_only() {
        let d = temp_dir("safari-listing");
        std::fs::write(d.join("FileSystemWritableStream.lloyd"), vec![0u8; 1000]).unwrap();
        let blobs = d.join("BlobRegistryFiles-acme");
        std::fs::create_dir_all(blobs.join("inner")).unwrap();
        std::fs::write(blobs.join("a"), vec![0u8; 30]).unwrap();
        std::fs::write(blobs.join("inner/b"), vec![0u8; 12]).unwrap();
        let outside = temp_dir("safari-listing-outside");
        std::fs::write(outside.join("big"), vec![0u8; 5000]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, blobs.join("link")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, d.join("beachape")).unwrap();
        let mut listing = list_folder(&d);
        listing.sort_by(|a, b| a.name.cmp(&b.name));
        let mut wanted = vec![
            entry("BlobRegistryFiles-acme", EntryKind::Folder, 42),
            entry("FileSystemWritableStream.lloyd", EntryKind::File, 1000),
        ];
        if cfg!(unix) {
            wanted.push(entry("beachape", EntryKind::Other, 0));
        }
        assert_eq!(listing, wanted);
        assert_eq!(list_folder(&d.join("missing")), Vec::new());
        std::fs::remove_dir_all(d).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn only_safari_on_macos_lists_safaris_folder() {
        let not_read = || -> Option<PathBuf> { panic!("the temporary folder was read") };
        assert!(watch_safari_data(Browser::Chrome, true, not_read).is_none());
        assert!(watch_safari_data(Browser::Safari, false, not_read).is_none());
        assert!(watch_safari_data(Browser::Safari, true, || None).is_none());
        let d = temp_dir("safari-watch");
        let watch = watch_safari_data(Browser::Safari, true, || Some(d.clone()));
        assert_eq!(
            watch.as_ref().map(|watch| watch.folder.clone()),
            Some(d.join(SAFARI_FOLDER))
        );
        drop(watch);
        std::fs::remove_dir_all(d).unwrap();
    }

    thread_local! {
        static PRINTED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn record(line: &str) {
        PRINTED.with_borrow_mut(|lines| lines.push(line.to_string()));
    }

    /// The drop of a [`SafariData`] lists the folder again and gives the
    /// line of what the run left.
    #[test]
    fn the_drop_gives_the_line_of_what_the_run_left() {
        let d = temp_dir("safari-drop");
        let watch = SafariData {
            folder: d.clone(),
            before: list_folder(&d),
            print: record,
        };
        std::fs::write(
            d.join("FileSystemWritableStreamlloyd"),
            vec![0u8; 2_000_000],
        )
        .unwrap();
        drop(watch);
        std::fs::remove_dir_all(&d).unwrap();
        let printed = PRINTED.with_borrow(Clone::clone);
        assert_eq!(printed.len(), 1, "{printed:?}");
        assert!(
            printed[0]
                .starts_with("browser data: Safari left 1 swap file (2.0 MB) and 0 blob folders"),
            "{printed:?}"
        );
    }

    /// The process list of this user's processes names this test process,
    /// with its command line.
    #[test]
    fn the_command_lines_name_this_process() {
        let ps = command_lines().expect("ps lists the processes");
        let this = std::process::id().to_string();
        let line = ps
            .lines()
            .map(str::trim_start)
            .find(|line| line.split_whitespace().next() == Some(this.as_str()))
            .unwrap_or_else(|| panic!("no line for {this}"));
        assert!(line.contains("xtask"), "{line}");
    }

    /// `getconf DARWIN_USER_TEMP_DIR` gives an absolute folder that exists,
    /// on macOS; elsewhere, none.
    #[test]
    fn the_user_temp_dir_is_a_folder_on_macos_only() {
        let temp = darwin_user_temp_dir();
        if cfg!(target_os = "macos") {
            let temp = temp.expect("a folder on macOS");
            assert!(temp.is_absolute() && temp.is_dir(), "{}", temp.display());
        } else {
            assert_eq!(temp, None);
        }
    }
}
