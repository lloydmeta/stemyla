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
//! (TD-96).

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
}
