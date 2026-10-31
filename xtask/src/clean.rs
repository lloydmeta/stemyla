//! `xtask clean [--yes] [--also <path>]...`: lists what runs leave in the
//! repo, with sizes, and changes nothing.  `--yes` removes only the items
//! that it can prove safe.
//!
//! The code has two parts, so the tests can check the rules without the
//! network.  [`gather`] reads the repo with `git` and the file system, and
//! takes the facts from outside the repo ([`Facts`]) from its argument.
//! [`plan`] is a pure function: it gives each item a proof or a reason.
//!
//! `plans/active/harness-phase-3b-m1.md` (step 1) gives the rules for each
//! kind of item.

use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub const USAGE: &str = "usage: xtask clean [--yes] [--also <path>]...";

/// The exit code of a usage error and of a refused `--also` path.
const REFUSED: i32 = 2;

/// The exit code of a failure.
const FAILED: i32 = 1;

/// The local branches that always stay.
const KEPT_BRANCHES: &[&str] = &["main", "gh-pages"];

/// The git words that remove a worktree.  No `--force`: git then refuses
/// a worktree that changed after the survey.
const WORKTREE_REMOVE: [&str; 2] = ["worktree", "remove"];

/// The time that `--yes` waits for a stopped `xtask serve` to end.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// What the code reads from outside the repo's files.  `None` means that
/// the fact is unknown: an item that needs it is kept.
pub struct Facts {
    /// The head branch of each open PR.
    pub open_prs: Option<Vec<String>>,
    /// The head branch and the `headRefOid` of each merged PR.
    pub merged_prs: Option<Vec<(String, String)>>,
    /// Each open file and working folder of each process, canonical.
    pub open_paths: Option<Vec<PathBuf>>,
    /// What runs as a pid, and if it listens on a TCP port.
    pub process: fn(u32, u16) -> Process,
}

impl Facts {
    /// The real facts: `gh` in `root`, `lsof`, and `ps` for each pid.
    pub fn read(root: &Path) -> Facts {
        Facts {
            open_prs: open_prs(root),
            merged_prs: merged_prs(root),
            open_paths: open_paths(),
            process: probe,
        }
    }
}

/// What runs as the pid of a pid file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Process {
    /// No process, or a zombie.
    Ended,
    /// A process that does not run `xtask serve`.
    Other,
    /// `xtask serve` that does not listen on the port of the file.
    NotListening,
    /// `xtask serve` that listens on the port of the file.
    Serve,
    /// A fact that the code cannot read, by its name.
    Unknown(String),
}

/// The repo, as `gather` reads it.
#[derive(Debug, Clone, Default)]
pub struct Survey {
    /// The repo root, canonical.
    pub root: PathBuf,
    pub worktrees: Vec<Worktree>,
    pub branches: Vec<Branch>,
    pub run_folders: Vec<RunFolder>,
    /// `target/e2e/` and `app/dist/`, when they exist.
    pub builds: Vec<Folder>,
    pub pid_files: Vec<PidFile>,
    /// The `--also` folders, canonical.
    pub scratch: Vec<Folder>,
    pub open_prs: Option<Vec<String>>,
    pub merged_prs: Option<Vec<(String, String)>>,
    pub open_paths: Option<Vec<PathBuf>>,
    /// The size of `target/`.
    pub target_size: u64,
}

/// One record of `git worktree list`.
#[derive(Debug, Clone, Default)]
pub struct Worktree {
    /// Canonical; for a folder that is gone, its nearest folder that
    /// exists is canonical.
    pub path: PathBuf,
    pub head: String,
    pub short: String,
    /// The short branch name, or `None` for a detached `HEAD`.
    pub branch: Option<String>,
    pub locked: bool,
    pub prunable: bool,
    /// Read only for a worktree under the two roots that is not prunable.
    /// `None` if `git status` failed.
    pub status: Option<Status>,
    /// The refs that contain `head`.  Read only under the two roots.
    pub refs: Vec<String>,
    /// If `head` is an ancestor of `origin/main`.  `None` if unknown.
    pub merged: Option<bool>,
    pub size: u64,
}

/// What `git status` shows in a worktree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Status {
    /// A change or an untracked file.
    pub changes: bool,
    /// An ignored path outside a `target/` folder at the top.
    pub ignored: bool,
}

/// A local branch.
#[derive(Debug, Clone, Default)]
pub struct Branch {
    pub name: String,
    pub tip: String,
    pub short: String,
    /// If the tip is an ancestor of `origin/main`.  `None` without it.
    pub ancestor: Option<bool>,
    /// If the tip's tree is the tree of a commit on `origin/main` that the
    /// tip does not hold (`<tip>..origin/main`).
    pub tree_on_main: Option<bool>,
}

/// A folder `target/review/run/<name>/`.
#[derive(Debug, Clone)]
pub struct RunFolder {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub log: Log,
    /// Each round folder with `state.json` and no `findings.md`.
    pub open_rounds: Vec<u32>,
}

/// The `log.md` of a run folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Log {
    Missing,
    /// Its blob is the blob of a file in `docs/reviews/`.
    Copied,
    NotCopied,
}

/// A file `target/xtask-serve/<port>.pid`.
#[derive(Debug, Clone)]
pub struct PidFile {
    pub path: PathBuf,
    pub port: u16,
    /// `None` if the file holds no pid of 2 or more.
    pub pid: Option<u32>,
    pub process: Process,
}

/// A folder and its size.
#[derive(Debug, Clone)]
pub struct Folder {
    pub path: PathBuf,
    pub size: u64,
}

/// The kinds of item, in the order that the output and `--yes` use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    PidFile,
    Worktree,
    Branch,
    RunFolder,
    Build,
    Scratch,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::PidFile => "pid-file",
            Kind::Worktree => "worktree",
            Kind::Branch => "branch",
            Kind::RunFolder => "run-folder",
            Kind::Build => "build",
            Kind::Scratch => "scratch",
        }
    }
}

/// What `--yes` does with an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Removed, with the proof that makes it safe.
    Removable(String),
    /// Never removed, with the reason.
    Kept(String),
}

/// One thing that `xtask clean` names.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    /// The path, relative to the repo root when it is under it, or the
    /// branch name.
    pub name: String,
    /// The absolute path of a folder or a file.
    pub path: Option<PathBuf>,
    /// The size of a folder.
    pub size: Option<u64>,
    /// The tip of a branch.
    pub tip: Option<String>,
    /// The pid of a running `xtask serve`, which `--yes` stops.
    pub pid: Option<u32>,
    /// The current branch of a worktree.
    pub branch: Option<String>,
    /// The name of the run folder of a worktree (see `run_name`).
    pub run: Option<String>,
    pub verdict: Verdict,
}

impl Item {
    fn new(kind: Kind, name: String, verdict: Verdict) -> Item {
        Item {
            kind,
            name,
            path: None,
            size: None,
            tip: None,
            pid: None,
            branch: None,
            run: None,
            verdict,
        }
    }

    fn folder(kind: Kind, root: &Path, folder: &Folder, verdict: Verdict) -> Item {
        Item {
            path: Some(folder.path.clone()),
            size: Some(folder.size),
            ..Item::new(kind, shown(root, &folder.path), verdict)
        }
    }

    pub fn removable(&self) -> bool {
        matches!(self.verdict, Verdict::Removable(_))
    }

    /// `<kind> <name> (<size or tip>): <proof or reason>`.
    pub fn line(&self) -> String {
        let note = match (&self.verdict, self.size, &self.tip) {
            (Verdict::Removable(_), Some(size), _) => format!(" ({})", human_size(size)),
            (_, _, Some(tip)) => format!(" ({tip})"),
            _ => String::new(),
        };
        let why = match &self.verdict {
            Verdict::Removable(why) | Verdict::Kept(why) => why,
        };
        format!("{} {}{note}: {why}", self.kind.word(), self.name)
    }
}

/// The repo root of the folder `dir`: the path of the first record of
/// `git worktree list`, canonical.  It is the main worktree, also from a
/// linked worktree.
pub fn repo_root(dir: &Path) -> Result<PathBuf> {
    let listing = git(dir, &["worktree", "list", "--porcelain", "-z"])?;
    let first = listing
        .split('\0')
        .next()
        .and_then(|field| field.strip_prefix("worktree "))
        .context("git worktree list gave no worktree")?;
    Ok(canonical(Path::new(first)))
}

/// Decides each item of `s`.
///
/// The worktrees come first.  The branches and the run folders then use
/// the worktrees that stay, so a branch that only a removed worktree held
/// goes in the same run.
pub fn plan(s: &Survey) -> Vec<Item> {
    // The listed worktree paths are canonical, also through a linked
    // `target/`, so the two roots are too.
    let review = canonical(&s.root.join("target/review"));
    let claude = canonical(&s.root.join(".claude/worktrees"));
    let under_roots = |path: &Path| path.starts_with(&review) || path.starts_with(&claude);

    // The branches with a proof, before the rule of the current branch.
    let proofs: BTreeMap<&str, String> = s
        .branches
        .iter()
        .filter(|b| !KEPT_BRANCHES.contains(&b.name.as_str()))
        .filter_map(|b| branch_proof(s, b).ok().map(|p| (b.name.as_str(), p)))
        .collect();

    // A worktree outside the two roots always stays.  Its run folder can
    // hold an open round, which keeps the reviewers' worktrees.
    let outside: Vec<&Worktree> = s
        .worktrees
        .iter()
        .filter(|w| !under_roots(&w.path))
        .collect();
    let open_rounds: Vec<(u32, &str)> = outside
        .iter()
        .filter_map(|w| s.run_folders.iter().find(|r| r.name == run_name(w)))
        .flat_map(|r| r.open_rounds.iter().map(|n| (*n, r.name.as_str())))
        .collect();

    let mut decided: Vec<(&Worktree, Verdict)> = s
        .worktrees
        .iter()
        .filter(|w| under_roots(&w.path))
        .map(|w| {
            let verdict = worktree_verdict(s, w, &review, &claude, &proofs, &open_rounds);
            (w, verdict)
        })
        .collect();
    // `git worktree remove` deletes each worktree under the one that it
    // removes.  So a worktree that holds a worktree that stays stays too.
    // One pass is enough.  A path under a worktree is also under each
    // worktree above it, so each of those holds the worktree that stays.  A
    // removable worktree is not in `staying_paths`, so it does not hold
    // itself.
    let staying_paths: Vec<PathBuf> = outside
        .iter()
        .map(|w| w.path.clone())
        .chain(
            decided
                .iter()
                .filter(|(_, v)| !matches!(v, Verdict::Removable(_)))
                .map(|(w, _)| w.path.clone()),
        )
        .collect();
    for (w, verdict) in &mut decided {
        if matches!(verdict, Verdict::Removable(_))
            && staying_paths.iter().any(|p| p.starts_with(&w.path))
        {
            *verdict = Verdict::Kept("holds a worktree that stays".to_string());
        }
    }

    let mut items = Vec::new();
    let mut staying: Vec<&Worktree> = outside.clone();
    for (w, verdict) in decided {
        if !matches!(verdict, Verdict::Removable(_)) {
            staying.push(w);
        }
        items.push(Item {
            path: Some(w.path.clone()),
            size: Some(w.size),
            branch: w.branch.clone(),
            run: Some(run_name(w)),
            ..Item::new(Kind::Worktree, shown(&s.root, &w.path), verdict)
        });
    }

    let held_branches: BTreeSet<&str> =
        staying.iter().filter_map(|w| w.branch.as_deref()).collect();
    let held_runs: BTreeSet<String> = staying.iter().map(|w| run_name(w)).collect();

    for b in &s.branches {
        let verdict = if KEPT_BRANCHES.contains(&b.name.as_str()) {
            Verdict::Kept("main and gh-pages stay".to_string())
        } else if held_branches.contains(b.name.as_str()) {
            Verdict::Kept("the current branch of a worktree".to_string())
        } else {
            match branch_proof(s, b) {
                Ok(proof) => Verdict::Removable(proof),
                Err(reason) => Verdict::Kept(reason),
            }
        };
        items.push(Item {
            tip: Some(b.short.clone()),
            ..Item::new(Kind::Branch, b.name.clone(), verdict)
        });
    }

    for r in &s.run_folders {
        let verdict = run_folder_verdict(s, r, &held_runs);
        let folder = Folder {
            path: r.path.clone(),
            size: r.size,
        };
        items.push(Item::folder(Kind::RunFolder, &s.root, &folder, verdict));
    }

    for b in &s.builds {
        let verdict = Verdict::Removable("build output".to_string());
        items.push(Item::folder(Kind::Build, &s.root, b, verdict));
    }

    for p in &s.pid_files {
        let verdict = pid_verdict(p);
        let running = p.process == Process::Serve;
        items.push(Item {
            path: Some(p.path.clone()),
            pid: if running { p.pid } else { None },
            ..Item::new(Kind::PidFile, shown(&s.root, &p.path), verdict)
        });
    }

    for f in &s.scratch {
        let verdict = scratch_verdict(s, f);
        items.push(Item::folder(Kind::Scratch, &s.root, f, verdict));
    }

    items.sort_by_key(|item| item.kind);
    items
}

/// The proof that a branch is merged, or the reason that it has none.
fn branch_proof(s: &Survey, b: &Branch) -> Result<String, String> {
    if b.ancestor == Some(true) {
        return Ok("its tip is an ancestor of origin/main".to_string());
    }
    if b.tree_on_main == Some(true) {
        return Ok("its tree is the tree of a commit on origin/main".to_string());
    }
    match &s.merged_prs {
        None => Err("cannot read merged PRs".to_string()),
        Some(prs)
            if prs
                .iter()
                .any(|(head, oid)| *head == b.name && *oid == b.tip) =>
        {
            Ok("a merged PR has this branch and tip".to_string())
        }
        Some(_) => Err("no proof that it is merged".to_string()),
    }
}

/// The run folder of a worktree: its branch with `/` as `-`, or
/// `detached-<short sha>`.
fn run_name(w: &Worktree) -> String {
    match &w.branch {
        Some(branch) => branch.replace('/', "-"),
        None => format!("detached-{}", w.short),
    }
}

fn worktree_verdict(
    s: &Survey,
    w: &Worktree,
    review: &Path,
    claude: &Path,
    proofs: &BTreeMap<&str, String>,
    open_rounds: &[(u32, &str)],
) -> Verdict {
    let kept = |reason: String| Verdict::Kept(reason);
    // Removing a worktree keeps its branch.  A branch that this run can
    // delete keeps only its tip, which its proof covers.
    let on_ref = w.refs.iter().any(|r| match r.strip_prefix("refs/heads/") {
        Some(b) if proofs.contains_key(b) => {
            s.branches.iter().any(|x| x.name == b && x.tip == w.head)
        }
        _ => true,
    });
    if w.locked {
        return kept("locked".to_string());
    }
    if w.prunable {
        return if on_ref {
            Verdict::Removable("prunable: its folder is gone".to_string())
        } else {
            kept(format!(
                "prunable, and its HEAD {} is on no ref: run `git worktree repair`, or prune it by hand",
                w.short
            ))
        };
    }
    match w.status {
        None => return kept("cannot read its status".to_string()),
        Some(status) if status.changes => return kept("has changes".to_string()),
        Some(status) if status.ignored => return kept("holds ignored files".to_string()),
        Some(_) => {}
    }
    if w.path.parent() == Some(review) {
        let name = w.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let round = name.split('-').next().and_then(|r| r.parse::<u32>().ok());
        if let Some((round, run)) = open_rounds.iter().find(|(r, _)| Some(*r) == round) {
            return kept(format!("round {round} of {run} has no findings.md"));
        }
    }
    match &s.open_paths {
        None => return kept("cannot read processes".to_string()),
        Some(paths) if paths.iter().any(|p| p.starts_with(&w.path)) => {
            return kept("a process uses it".to_string());
        }
        Some(_) => {}
    }
    if !on_ref {
        return kept(format!("its HEAD {} is on no ref", w.short));
    }
    if w.path.starts_with(claude) {
        let branch_proven = w.branch.as_deref().is_some_and(|b| proofs.contains_key(b));
        if w.merged == Some(true) || branch_proven {
            return Verdict::Removable("clean, unused and merged".to_string());
        }
        return kept("not merged".to_string());
    }
    Verdict::Removable("clean and unused".to_string())
}

fn run_folder_verdict(s: &Survey, r: &RunFolder, held_runs: &BTreeSet<String>) -> Verdict {
    let kept = |reason: &str| Verdict::Kept(reason.to_string());
    if held_runs.contains(&r.name) {
        return kept("the current branch of a worktree");
    }
    if let Some(round) = r.open_rounds.first() {
        return Verdict::Kept(format!("round {round} is open"));
    }
    match r.log {
        Log::Missing => return kept("no log.md"),
        Log::NotCopied => return kept("log.md has no copy in docs/reviews/"),
        Log::Copied => {}
    }
    match &s.open_prs {
        None => kept("cannot read open PRs"),
        Some(prs) if prs.iter().any(|head| head.replace('/', "-") == r.name) => {
            kept("a branch with an open PR")
        }
        Some(_) => Verdict::Removable("its log.md has a copy in docs/reviews/".to_string()),
    }
}

fn pid_verdict(p: &PidFile) -> Verdict {
    let Some(pid) = p.pid else {
        return Verdict::Removable("stale: it holds no pid".to_string());
    };
    let port = p.port;
    match &p.process {
        Process::Serve => {
            Verdict::Removable(format!("a running xtask serve on port {port} (pid {pid})"))
        }
        Process::Ended => Verdict::Removable(format!("stale: pid {pid} has ended")),
        Process::Other => Verdict::Removable(format!(
            "stale: pid {pid} runs another program, which stays"
        )),
        Process::NotListening => Verdict::Removable(format!(
            "stale: pid {pid} does not listen on port {port}, and stays"
        )),
        Process::Unknown(fact) => Verdict::Kept(format!("cannot read {fact}")),
    }
}

fn scratch_verdict(s: &Survey, f: &Folder) -> Verdict {
    match &s.open_paths {
        None => return Verdict::Kept("cannot read processes".to_string()),
        Some(paths) if paths.iter().any(|p| p.starts_with(&f.path)) => {
            return Verdict::Kept("a process uses it".to_string());
        }
        Some(_) => {}
    }
    if s.worktrees.iter().any(|w| w.path.starts_with(&f.path)) {
        return Verdict::Kept("holds a worktree of this repo".to_string());
    }
    Verdict::Removable("a scratch root named with --also".to_string())
}

/// The lines of the list.  `also` holds the `--also` arguments.
pub fn report(items: &[Item], target_size: u64, also: &[String]) -> Vec<String> {
    let removable: Vec<&Item> = items.iter().filter(|i| i.removable()).collect();
    let kept: Vec<&Item> = items.iter().filter(|i| !i.removable()).collect();
    let mut lines = Vec::new();
    if removable.is_empty() {
        lines.push("nothing to remove".to_string());
    } else {
        // A removable worktree under another one is in that one's size.
        let worktrees: Vec<&Path> = removable
            .iter()
            .filter(|i| i.kind == Kind::Worktree)
            .filter_map(|i| i.path.as_deref())
            .collect();
        let nested = |i: &Item| {
            i.kind == Kind::Worktree
                && i.path
                    .as_deref()
                    .is_some_and(|p| worktrees.iter().any(|w| *w != p && p.starts_with(w)))
        };
        let total: u64 = removable
            .iter()
            .filter(|i| !nested(i))
            .filter_map(|i| i.size)
            .sum();
        lines.push(format!(
            "can remove ({} items, {}):",
            removable.len(),
            human_size(total)
        ));
        lines.extend(removable.iter().map(|i| format!("  {}", i.line())));
    }
    lines.push(format!("keeps ({} items):", kept.len()));
    lines.extend(kept.iter().map(|i| format!("  {}", i.line())));
    lines.push(format!(
        "target/ holds {}.  `cargo clean` removes it: run it by hand.",
        human_size(target_size)
    ));
    if !removable.is_empty() {
        let also: String = also.iter().map(|p| format!(" --also {p}")).collect();
        lines.push(format!(
            "run `cargo run -p xtask -- clean --yes{also}` to remove the items above."
        ));
    }
    lines
}

/// A size in binary units.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// The options of the command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub yes: bool,
    pub also: Vec<String>,
}

pub fn parse_args(args: &[String]) -> Option<Options> {
    let mut options = Options::default();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--yes" => options.yes = true,
            "--also" => options.also.push(words.next()?.clone()),
            _ => return None,
        }
    }
    Some(options)
}

/// The canonical form of a valid `--also` path, or why it is refused.
///
/// It must be an absolute folder under the temporary folder, and not the
/// temporary folder itself.  It must not hold the repo root or be in it.
/// It must not be in a worktree of the repo, from `worktrees`.
pub fn check_also(arg: &str, root: &Path, worktrees: &[PathBuf]) -> Result<PathBuf, String> {
    let path = Path::new(arg);
    if !path.is_absolute() {
        return Err(format!("--also {arg}: the path must be absolute"));
    }
    if !path.is_dir() {
        return Err(format!("--also {arg}: no such folder"));
    }
    let path = canonical(path);
    let temps: Vec<PathBuf> = [
        std::env::temp_dir(),
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
    ]
    .iter()
    .filter_map(|t| t.canonicalize().ok())
    .collect();
    if !temps.iter().any(|t| path.starts_with(t) && path != *t) {
        return Err(format!(
            "--also {arg}: the path must be in the temporary folder, and not be it"
        ));
    }
    if root.starts_with(&path) || path.starts_with(root) {
        return Err(format!(
            "--also {arg}: the path must not hold the repo or be in it"
        ));
    }
    if let Some(worktree) = worktrees.iter().find(|w| path.starts_with(w)) {
        return Err(format!(
            "--also {arg}: the path is in the worktree {}",
            worktree.display()
        ));
    }
    Ok(path)
}

/// Runs `xtask clean` from the shell, and returns the exit code.
pub fn main(args: &[String]) -> i32 {
    let Some(options) = parse_args(args) else {
        eprintln!("{USAGE}");
        return REFUSED;
    };
    let root = match std::env::current_dir()
        .context("reading the current folder")
        .and_then(|dir| repo_root(&dir))
    {
        Ok(root) => root,
        Err(e) => {
            eprintln!("{e:#}");
            return FAILED;
        }
    };
    let worktrees: Vec<PathBuf> = match git(&root, &["worktree", "list", "--porcelain", "-z"]) {
        Ok(listing) => parse_worktrees(&listing)
            .into_iter()
            .map(|w| w.path)
            .collect(),
        Err(e) => {
            eprintln!("{e:#}");
            return FAILED;
        }
    };
    let mut also = Vec::new();
    for arg in &options.also {
        match check_also(arg, &root, &worktrees) {
            Ok(path) => also.push(path),
            Err(e) => {
                eprintln!("{e}");
                return REFUSED;
            }
        }
    }
    match run(&root, &also, &options) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e:#}");
            FAILED
        }
    }
}

fn run(root: &Path, also: &[PathBuf], options: &Options) -> Result<i32> {
    let survey = gather(root, &Facts::read(root), also)?;
    let items = plan(&survey);
    for line in report(&items, survey.target_size, &options.also) {
        println!("{line}");
    }
    if !options.yes {
        return Ok(0);
    }
    Ok(if remove(root, also, &items)? {
        0
    } else {
        FAILED
    })
}

/// `--yes`: stops each running `xtask serve`, then gathers again and
/// removes each item that is removable in the new survey.  Returns `false`
/// if one failed.
fn remove(root: &Path, also: &[PathBuf], first: &[Item]) -> Result<bool> {
    let mut ok = true;
    let mut fail = |item: &Item, error: &str| {
        println!("failed {}: {error}", item.line());
        ok = false;
    };
    for item in first
        .iter()
        .filter(|i| i.kind == Kind::PidFile && i.removable())
    {
        if let Some(pid) = item.pid
            && let Err(e) = stop(pid)
        {
            fail(item, &e);
            continue;
        }
        let path = item.path.as_deref().context("a pid file has a path")?;
        match std::fs::remove_file(path) {
            Ok(()) => println!("removed {}", item.line()),
            Err(e) => fail(item, &e.to_string()),
        }
    }

    // A worktree that a stopped server used is free now.
    let survey = gather(root, &Facts::read(root), also)?;
    let items = plan(&survey);
    let removable = |kind: Kind| {
        items
            .iter()
            .filter(move |i| i.kind == kind && i.removable())
    };

    // The branches and run folders that a worktree which stays still holds.
    let mut stuck: BTreeSet<String> = BTreeSet::new();
    let mut stuck_runs: BTreeSet<String> = BTreeSet::new();
    for item in removable(Kind::Worktree) {
        let path = item.path.as_deref().context("a worktree has a path")?;
        match remove_worktree(root, path) {
            Ok(_) => println!("removed {}", item.line()),
            Err(e) => {
                fail(item, &format!("{e:#}"));
                stuck.extend(item.branch.clone());
                stuck_runs.extend(item.run.clone());
            }
        }
    }
    for item in removable(Kind::Branch) {
        if stuck.contains(&item.name) {
            fail(item, "its worktree stays");
            continue;
        }
        match git(root, &["branch", "-D", &item.name]) {
            Ok(_) => println!("removed {}", item.line()),
            Err(e) => fail(item, &format!("{e:#}")),
        }
    }
    for kind in [Kind::RunFolder, Kind::Build, Kind::Scratch] {
        for item in removable(kind) {
            let path = item.path.as_deref().context("a folder has a path")?;
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if kind == Kind::RunFolder && stuck_runs.contains(name) {
                fail(item, "its worktree stays");
                continue;
            }
            match std::fs::remove_dir_all(path) {
                Ok(()) => println!("removed {}", item.line()),
                Err(e) => fail(item, &e.to_string()),
            }
        }
    }
    Ok(ok)
}

/// Removes the worktree at `path` with `git worktree remove`, with no
/// `--force`.  So git refuses a worktree with changes, also one that
/// changed after the survey.
pub fn remove_worktree(root: &Path, path: &Path) -> Result<()> {
    let path = path.to_str().context("a worktree path is UTF-8")?;
    let mut args = WORKTREE_REMOVE.to_vec();
    args.push(path);
    git(root, &args).map(|_| ())
}

/// Sends SIGTERM to `pid`, and waits for it to end.
fn stop(pid: u32) -> Result<(), String> {
    let out = Command::new("kill")
        .arg(pid.to_string())
        .output()
        .map_err(|e| format!("kill {pid}: {e}"))?;
    if !out.status.success() && !ended(pid) {
        return Err(format!(
            "kill {pid}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let start = Instant::now();
    while start.elapsed() < STOP_WAIT {
        if ended(pid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "pid {pid} still runs after {} s",
        STOP_WAIT.as_secs()
    ))
}

/// If `pid` has no process, or a zombie.
fn ended(pid: u32) -> bool {
    match ask(Command::new("ps").args(["-p", &pid.to_string(), "-o", "stat="])) {
        Answer::No => true,
        Answer::Yes(stat) => stat.trim_start().starts_with('Z'),
        Answer::Unknown => false,
    }
}

/// Reads the repo at `root` (canonical), with `facts` from outside it.
pub fn gather(root: &Path, facts: &Facts, also: &[PathBuf]) -> Result<Survey> {
    let origin_main = "refs/remotes/origin/main";
    let has_main = git(
        root,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{origin_main}^{{commit}}"),
        ],
    )
    .is_ok();
    // A squash merge puts the tree of the tip on a commit of main that the
    // branch does not hold.  A commit that the branch holds does not count,
    // so a branch that reverts main to an older tree has no proof.
    let tree_on_main = |tip: &str, tree: &str| -> Result<Option<bool>> {
        if !has_main {
            return Ok(None);
        }
        let range = format!("{tip}..{origin_main}");
        let trees = git(root, &["log", "--format=%T", &range])?;
        Ok(Some(trees.lines().any(|t| t == tree)))
    };
    let ancestor = |sha: &str| -> Result<Option<bool>> {
        if !has_main {
            return Ok(None);
        }
        is_ancestor(root, sha, origin_main).map(Some)
    };

    let mut worktrees = parse_worktrees(&git(root, &["worktree", "list", "--porcelain", "-z"])?);
    let mut branches = Vec::new();
    for line in git(
        root,
        &[
            "for-each-ref",
            "--format=%(objectname)%09%(tree)%09%(refname)",
            "refs/heads",
        ],
    )?
    .lines()
    {
        let mut fields = line.splitn(3, '\t');
        let (Some(tip), Some(tree), Some(name)) = (fields.next(), fields.next(), fields.next())
        else {
            bail!("cannot read the branch line {line}");
        };
        let name = name.strip_prefix("refs/heads/").unwrap_or(name);
        branches.push(Branch {
            name: name.to_string(),
            tip: tip.to_string(),
            short: String::new(),
            ancestor: ancestor(tip)?,
            tree_on_main: tree_on_main(tip, tree)?,
        });
    }
    let shas: Vec<String> = worktrees
        .iter()
        .map(|w| w.head.clone())
        .chain(branches.iter().map(|b| b.tip.clone()))
        .filter(|sha| !sha.is_empty())
        .collect();
    let shorts = short_shas(root, &shas)?;
    for b in &mut branches {
        b.short = shorts.get(&b.tip).cloned().unwrap_or_default();
    }

    let review = canonical(&root.join("target/review"));
    let claude = canonical(&root.join(".claude/worktrees"));
    for w in &mut worktrees {
        w.short = shorts.get(&w.head).cloned().unwrap_or_default();
        if !(w.path.starts_with(&review) || w.path.starts_with(&claude)) || w.head.is_empty() {
            continue;
        }
        w.refs = git(
            root,
            &["for-each-ref", "--contains", &w.head, "--format=%(refname)"],
        )?
        .lines()
        .map(ToString::to_string)
        .collect();
        w.merged = ancestor(&w.head)?;
        if !w.prunable {
            w.status = worktree_status(&w.path);
            w.size = dir_size(&w.path);
        }
    }

    let copies = review_blobs(root, has_main.then_some(origin_main))?;
    let mut run_folders = Vec::new();
    for path in subfolders(&root.join("target/review/run")) {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let log_path = path.join("log.md");
        let log = if !log_path.is_file() {
            Log::Missing
        } else {
            let blob = git(root, &["hash-object", "--", path_str(&log_path)?])?;
            if copies.contains(blob.trim()) {
                Log::Copied
            } else {
                Log::NotCopied
            }
        };
        let open_rounds = subfolders(&path)
            .iter()
            .filter(|round| {
                round.join("state.json").is_file() && !round.join("findings.md").exists()
            })
            .filter_map(|round| round.file_name()?.to_str()?.parse::<u32>().ok())
            .collect();
        run_folders.push(RunFolder {
            name,
            size: dir_size(&path),
            path,
            log,
            open_rounds,
        });
    }

    let builds = [root.join("target/e2e"), root.join("app/dist")]
        .into_iter()
        .filter(|p| p.symlink_metadata().is_ok_and(|m| m.is_dir()))
        .map(|path| Folder {
            size: dir_size(&path),
            path,
        })
        .collect();

    let mut pid_files = Vec::new();
    let serve_dir = root.join(crate::serve::PID_FOLDER);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&serve_dir)
        .map(|dir| dir.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    entries.sort();
    for path in entries {
        let port = (path.extension().and_then(|e| e.to_str()) == Some("pid"))
            .then(|| path.file_stem()?.to_str()?.parse::<u16>().ok())
            .flatten();
        let Some(port) = port.filter(|_| path.is_file()) else {
            continue;
        };
        let pid = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
            // `kill 0` and `kill 1` would signal more than one server.
            .filter(|pid| *pid >= 2);
        let process = match pid {
            Some(pid) => (facts.process)(pid, port),
            None => Process::Ended,
        };
        pid_files.push(PidFile {
            path,
            port,
            pid,
            process,
        });
    }

    let scratch = also
        .iter()
        .map(|path| Folder {
            path: path.clone(),
            size: dir_size(path),
        })
        .collect();

    Ok(Survey {
        root: root.to_path_buf(),
        worktrees,
        branches,
        run_folders,
        builds,
        pid_files,
        scratch,
        open_prs: facts.open_prs.clone(),
        merged_prs: facts.merged_prs.clone(),
        open_paths: facts.open_paths.clone(),
        target_size: dir_size(&root.join("target")),
    })
}

/// The records of `git worktree list --porcelain -z`, without a bare one.
fn parse_worktrees(listing: &str) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;
    let mut bare = false;
    for field in listing.split('\0') {
        if field.is_empty() {
            if let Some(w) = current.take().filter(|_| !bare) {
                worktrees.push(w);
            }
            bare = false;
            continue;
        }
        let (key, value) = field.split_once(' ').unwrap_or((field, ""));
        let w = current.get_or_insert_with(Worktree::default);
        match key {
            "worktree" => w.path = canonical(Path::new(value)),
            "HEAD" => w.head = value.to_string(),
            "branch" => {
                w.branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_string(),
                );
            }
            "locked" => w.locked = true,
            "prunable" => w.prunable = true,
            "bare" => bare = true,
            _ => {}
        }
    }
    if let Some(w) = current.filter(|_| !bare) {
        worktrees.push(w);
    }
    worktrees
}

/// `git status` in a worktree, or `None` if it fails.  It takes no lock,
/// and does not write the worktree's index.
fn worktree_status(path: &Path) -> Option<Status> {
    let out = git(
        path,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "-z",
            "--ignored",
            "--untracked-files=normal",
        ],
    )
    .ok()?;
    let mut status = Status::default();
    for entry in out.split('\0').filter(|e| !e.is_empty()) {
        match entry.strip_prefix("!! ") {
            Some(ignored) => {
                if !(ignored == "target/" || ignored.starts_with("target/")) {
                    status.ignored = true;
                }
            }
            None => status.changes = true,
        }
    }
    Some(status)
}

/// The blobs of the files under `docs/reviews/` at `HEAD` and at `main`.
fn review_blobs(root: &Path, main: Option<&str>) -> Result<BTreeSet<String>> {
    let mut blobs = BTreeSet::new();
    for rev in std::iter::once("HEAD").chain(main) {
        // A repo with no commit has no `HEAD`.
        let Ok(listing) = git(root, &["ls-tree", "-r", rev, "--", "docs/reviews/"]) else {
            continue;
        };
        for line in listing.lines() {
            let mut words = line.split_whitespace();
            if let (_, Some("blob"), Some(sha)) = (words.next(), words.next(), words.next()) {
                blobs.insert(sha.to_string());
            }
        }
    }
    Ok(blobs)
}

/// The short form of each sha, by its full form.
fn short_shas(root: &Path, shas: &[String]) -> Result<BTreeMap<String, String>> {
    let unique: BTreeSet<&str> = shas.iter().map(String::as_str).collect();
    if unique.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut args = vec!["log", "--no-walk=unsorted", "--format=%H %h"];
    args.extend(unique);
    let out = git(root, &args)?;
    Ok(out
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(full, short)| (full.to_string(), short.to_string()))
        .collect())
}

/// If `a` is an ancestor of `b`.
fn is_ancestor(root: &Path, a: &str, b: &str) -> Result<bool> {
    let out = Command::new("git")
        .args(["merge-base", "--is-ancestor", a, b])
        .current_dir(root)
        .output()
        .context("running git merge-base")?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => bail!(
            "git merge-base --is-ancestor {a} {b} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// The folders in `dir`, sorted; none if it does not exist.
fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    folders.sort();
    folders
}

/// The size of the files in `path`, from a walk that does not follow
/// symbolic links.
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| dir_size(&e.path()))
                .sum()
        })
        .unwrap_or(0)
}

/// The canonical form of `path`.  For a path that is gone, the nearest
/// folder above it that exists is canonical, and the rest is kept.
pub fn canonical(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => canonical(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// `path` relative to `root` when it is under it, else in full.  A path
/// through a linked `target/` or `.claude/` shows as under it.
fn shown(root: &Path, path: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(root) {
        return rel.display().to_string();
    }
    for folder in ["target", ".claude"] {
        if let Ok(rel) = path.strip_prefix(canonical(&root.join(folder))) {
            return Path::new(folder).join(rel).display().to_string();
        }
    }
    path.display().to_string()
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("{} is not UTF-8", path.display()))
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The answer of `ps` or `lsof` to a question about one pid.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Yes(String),
    No,
    Unknown,
}

/// Runs `command`.  `ps` and `lsof` exit 1 also when nothing matches, so
/// an exit of 0 or 1 with no output and nothing on stderr is "no".
fn ask(command: &mut Command) -> Answer {
    let Ok(out) = command.output() else {
        return Answer::Unknown;
    };
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let code = out.status.code();
    if code == Some(0) && !stdout.is_empty() {
        return Answer::Yes(stdout);
    }
    if matches!(code, Some(0 | 1)) && stdout.is_empty() && out.stderr.trim_ascii().is_empty() {
        return Answer::No;
    }
    Answer::Unknown
}

/// What runs as `pid`, and if it listens on `port`.
fn probe(pid: u32, port: u16) -> Process {
    let pid_arg = pid.to_string();
    match ask(Command::new("ps").args(["-p", &pid_arg, "-o", "stat="])) {
        Answer::Unknown => return Process::Unknown(format!("the state of pid {pid}")),
        Answer::No => return Process::Ended,
        Answer::Yes(stat) if stat.starts_with('Z') => return Process::Ended,
        Answer::Yes(_) => {}
    }
    match ask(Command::new("ps").args(["-p", &pid_arg, "-o", "command="])) {
        Answer::Unknown => return Process::Unknown(format!("the command line of pid {pid}")),
        Answer::No => return Process::Ended,
        Answer::Yes(command) if !runs_serve(&command) => return Process::Other,
        Answer::Yes(_) => {}
    }
    match ask(Command::new("lsof").args(lsof_listen_args(pid, port))) {
        Answer::Unknown => Process::Unknown(format!("the ports of pid {pid}")),
        Answer::No => Process::NotListening,
        Answer::Yes(_) => Process::Serve,
    }
}

/// The `lsof` words that list each open file and working folder.  `-w`
/// stops the warnings that Linux's `lsof` prints on stderr.
const LSOF_OPEN: [&str; 5] = ["-w", "-n", "-P", "-F", "n"];

/// The `lsof` words that give `pid` if it listens on TCP `port`.
fn lsof_listen_args(pid: u32, port: u16) -> Vec<String> {
    let words = ["-w", "-n", "-P", "-a", "-p"];
    words
        .iter()
        .map(ToString::to_string)
        .chain([
            pid.to_string(),
            format!("-iTCP:{port}"),
            "-sTCP:LISTEN".to_string(),
            "-t".to_string(),
        ])
        .collect()
}

/// If a command line has a word whose last path part is `xtask`, then the
/// word `serve`.
pub fn runs_serve(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    words.windows(2).any(|pair| {
        Path::new(pair[0]).file_name().and_then(|n| n.to_str()) == Some("xtask")
            && pair[1] == "serve"
    })
}

/// Each open file and working folder of each process, from `lsof`.
fn open_paths() -> Option<Vec<PathBuf>> {
    let out = Command::new("lsof").args(LSOF_OPEN).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // `lsof` exits 1 when it cannot read some processes; its output
    // counts when it names processes.
    if !text.lines().any(|line| line.starts_with('p')) {
        return None;
    }
    let paths: BTreeSet<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .filter(|name| name.starts_with('/'))
        .collect();
    Some(paths.into_iter().map(|p| canonical(Path::new(p))).collect())
}

/// The head branch of each open PR, from `gh`.
fn open_prs(root: &Path) -> Option<Vec<String>> {
    let prs = gh_list(root, "open", "headRefName", "500")?;
    prs.iter()
        .map(|pr| Some(pr.get("headRefName")?.as_str()?.to_string()))
        .collect()
}

/// The head branch and the `headRefOid` of each merged PR, from `gh`.
fn merged_prs(root: &Path) -> Option<Vec<(String, String)>> {
    let prs = gh_list(root, "merged", "headRefName,headRefOid", "1000")?;
    prs.iter()
        .map(|pr| {
            Some((
                pr.get("headRefName")?.as_str()?.to_string(),
                pr.get("headRefOid")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn gh_list(root: &Path, state: &str, fields: &str, limit: &str) -> Option<Vec<serde_json::Value>> {
    let out = Command::new("gh")
        .args([
            "pr", "list", "--state", state, "--json", fields, "--limit", limit,
        ])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/r";

    fn sha(n: u8) -> String {
        format!("{n:040}")
    }

    /// A survey of a repo with only its main worktree on `main`, with
    /// every fact known and empty.
    fn survey() -> Survey {
        Survey {
            root: PathBuf::from(ROOT),
            worktrees: vec![Worktree {
                path: PathBuf::from(ROOT),
                head: sha(1),
                short: "s1".to_string(),
                branch: Some("main".to_string()),
                ..Worktree::default()
            }],
            branches: vec![branch("main", 1)],
            open_prs: Some(Vec::new()),
            merged_prs: Some(Vec::new()),
            open_paths: Some(Vec::new()),
            ..Survey::default()
        }
    }

    fn branch(name: &str, tip: u8) -> Branch {
        Branch {
            name: name.to_string(),
            tip: sha(tip),
            short: format!("s{tip}"),
            ancestor: Some(false),
            tree_on_main: Some(false),
        }
    }

    /// A clean, unused worktree at `rel` under the root, on `head`, which
    /// `refs/heads/keep` contains.
    fn worktree(rel: &str, head: u8) -> Worktree {
        Worktree {
            path: Path::new(ROOT).join(rel),
            head: sha(head),
            short: format!("s{head}"),
            status: Some(Status::default()),
            refs: vec!["refs/heads/keep".to_string()],
            merged: Some(false),
            size: 10,
            ..Worktree::default()
        }
    }

    fn run_folder(name: &str, log: Log) -> RunFolder {
        RunFolder {
            name: name.to_string(),
            path: Path::new(ROOT).join("target/review/run").join(name),
            size: 5,
            log,
            open_rounds: Vec::new(),
        }
    }

    fn verdict(s: &Survey, kind: Kind, name: &str) -> Verdict {
        let items = plan(s);
        items
            .iter()
            .find(|i| i.kind == kind && i.name == name)
            .unwrap_or_else(|| panic!("no {} {name} in {items:?}", kind.word()))
            .verdict
            .clone()
    }

    fn removable(why: &str) -> Verdict {
        Verdict::Removable(why.to_string())
    }

    fn kept(why: &str) -> Verdict {
        Verdict::Kept(why.to_string())
    }

    // Branches.

    #[test]
    fn a_branch_whose_tip_is_an_ancestor_of_origin_main_goes() {
        let mut s = survey();
        s.branches.push(Branch {
            ancestor: Some(true),
            ..branch("ancestor", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Branch, "ancestor"),
            removable("its tip is an ancestor of origin/main")
        );
    }

    #[test]
    fn a_squash_merged_branch_goes() {
        let mut s = survey();
        s.branches.push(Branch {
            tree_on_main: Some(true),
            ..branch("squashed", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Branch, "squashed"),
            removable("its tree is the tree of a commit on origin/main")
        );
    }

    #[test]
    fn a_branch_with_a_merged_pr_of_its_tip_goes() {
        let mut s = survey();
        s.branches.push(branch("merged-pr", 2));
        s.merged_prs = Some(vec![("merged-pr".to_string(), sha(2))]);
        assert_eq!(
            verdict(&s, Kind::Branch, "merged-pr"),
            removable("a merged PR has this branch and tip")
        );
    }

    #[test]
    fn a_merged_pr_proves_a_branch_with_no_origin_main() {
        let mut s = survey();
        s.branches.push(Branch {
            ancestor: None,
            tree_on_main: None,
            ..branch("merged-pr", 2)
        });
        s.merged_prs = Some(vec![("merged-pr".to_string(), sha(2))]);
        assert_eq!(
            verdict(&s, Kind::Branch, "merged-pr"),
            removable("a merged PR has this branch and tip")
        );
    }

    #[test]
    fn a_branch_that_moved_on_after_its_merged_pr_stays() {
        let mut s = survey();
        s.branches.push(branch("moved-on", 3));
        s.merged_prs = Some(vec![("moved-on".to_string(), sha(2))]);
        assert_eq!(
            verdict(&s, Kind::Branch, "moved-on"),
            kept("no proof that it is merged")
        );
    }

    #[test]
    fn a_branch_stays_when_the_merged_prs_are_unknown() {
        let mut s = survey();
        s.branches.push(branch("merged-pr", 2));
        s.merged_prs = None;
        assert_eq!(
            verdict(&s, Kind::Branch, "merged-pr"),
            kept("cannot read merged PRs")
        );
    }

    #[test]
    fn main_and_gh_pages_stay_with_a_proof() {
        let mut s = survey();
        s.branches = vec![
            Branch {
                ancestor: Some(true),
                ..branch("main", 1)
            },
            Branch {
                ancestor: Some(true),
                ..branch("gh-pages", 2)
            },
        ];
        for name in ["main", "gh-pages"] {
            assert_eq!(
                verdict(&s, Kind::Branch, name),
                kept("main and gh-pages stay")
            );
        }
    }

    #[test]
    fn the_current_branch_of_a_worktree_that_stays_stays() {
        let mut s = survey();
        s.branches.push(Branch {
            ancestor: Some(true),
            ..branch("ancestor-wt", 2)
        });
        s.worktrees.push(Worktree {
            path: PathBuf::from("/elsewhere/wt"),
            branch: Some("ancestor-wt".to_string()),
            ..worktree("", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Branch, "ancestor-wt"),
            kept("the current branch of a worktree")
        );
    }

    #[test]
    fn the_branch_of_a_removed_worktree_goes_in_the_same_run() {
        let mut s = survey();
        s.branches.push(Branch {
            ancestor: Some(true),
            ..branch("agent-x", 2)
        });
        s.worktrees.push(Worktree {
            branch: Some("agent-x".to_string()),
            merged: Some(true),
            ..worktree(".claude/worktrees/agent-x", 2)
        });
        assert!(matches!(
            verdict(&s, Kind::Worktree, ".claude/worktrees/agent-x"),
            Verdict::Removable(_)
        ));
        assert!(matches!(
            verdict(&s, Kind::Branch, "agent-x"),
            Verdict::Removable(_)
        ));
    }

    #[test]
    fn the_branch_of_a_kept_worktree_under_a_root_stays() {
        let mut s = survey();
        s.branches.push(Branch {
            ancestor: Some(true),
            ..branch("agent-x", 2)
        });
        s.worktrees.push(Worktree {
            branch: Some("agent-x".to_string()),
            status: Some(Status {
                changes: true,
                ignored: false,
            }),
            ..worktree(".claude/worktrees/agent-x", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Branch, "agent-x"),
            kept("the current branch of a worktree")
        );
    }

    // Worktrees.

    #[test]
    fn a_clean_unused_worktree_on_a_ref_goes() {
        let mut s = survey();
        s.worktrees.push(worktree("target/review/1-reviewer-2", 1));
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-2"),
            removable("clean and unused")
        );
    }

    #[test]
    fn a_worktree_outside_the_two_roots_is_not_an_item() {
        let mut s = survey();
        s.worktrees.push(worktree("elsewhere/wt", 1));
        s.worktrees.push(worktree("target/other", 1));
        let items = plan(&s);
        assert!(items.iter().all(|i| i.kind != Kind::Worktree), "{items:?}");
    }

    #[test]
    fn a_locked_worktree_stays() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            locked: true,
            ..worktree("target/review/1-reviewer-1", 1)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-1"),
            kept("locked")
        );
    }

    #[test]
    fn a_worktree_with_changes_stays() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            status: Some(Status {
                changes: true,
                ignored: false,
            }),
            ..worktree("target/review/1-reviewer-1", 1)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-1"),
            kept("has changes")
        );
    }

    #[test]
    fn a_worktree_with_ignored_files_stays() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            status: Some(Status {
                changes: false,
                ignored: true,
            }),
            ..worktree("target/review/1-reviewer-3", 1)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-3"),
            kept("holds ignored files")
        );
    }

    #[test]
    fn a_worktree_whose_status_is_unknown_stays() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            status: None,
            ..worktree("target/review/1-reviewer-1", 1)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-1"),
            kept("cannot read its status")
        );
    }

    #[test]
    fn a_worktree_that_a_process_uses_stays() {
        let mut s = survey();
        s.worktrees.push(worktree("target/review/1-reviewer-1", 1));
        s.open_paths = Some(vec![PathBuf::from(
            "/r/target/review/1-reviewer-1/src/a.rs",
        )]);
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-1"),
            kept("a process uses it")
        );
    }

    #[test]
    fn a_process_in_a_folder_with_a_longer_name_does_not_use_the_worktree() {
        let mut s = survey();
        s.worktrees.push(worktree("target/review/1-reviewer-1", 1));
        s.open_paths = Some(vec![PathBuf::from("/r/target/review/1-reviewer-10/a.rs")]);
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-1"),
            removable("clean and unused")
        );
    }

    #[test]
    fn a_worktree_stays_when_the_processes_are_unknown() {
        let mut s = survey();
        s.worktrees.push(worktree("target/review/1-reviewer-1", 1));
        s.open_paths = None;
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-1"),
            kept("cannot read processes")
        );
    }

    #[test]
    fn a_worktree_of_an_open_round_of_a_worktree_that_stays_stays() {
        let mut s = survey();
        s.worktrees.push(worktree("target/review/2-reviewer-1", 1));
        s.worktrees.push(worktree("target/review/20-reviewer-1", 1));
        s.run_folders.push(RunFolder {
            open_rounds: vec![2],
            ..run_folder("main", Log::Missing)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/2-reviewer-1"),
            kept("round 2 of main has no findings.md")
        );
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/20-reviewer-1"),
            removable("clean and unused"),
            "round 2 does not hold round 20"
        );
    }

    #[test]
    fn an_open_round_of_a_detached_worktree_that_stays_holds_its_reviewers() {
        let mut s = survey();
        s.worktrees[0].branch = None;
        s.worktrees.push(worktree("target/review/2-reviewer-1", 1));
        s.run_folders.push(RunFolder {
            open_rounds: vec![2],
            ..run_folder("detached-s1", Log::Missing)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/2-reviewer-1"),
            kept("round 2 of detached-s1 has no findings.md")
        );
    }

    #[test]
    fn an_open_round_of_a_run_folder_of_no_worktree_holds_nothing() {
        let mut s = survey();
        s.worktrees.push(worktree("target/review/3-reviewer-1", 1));
        s.run_folders.push(RunFolder {
            open_rounds: vec![3],
            ..run_folder("other", Log::Missing)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/3-reviewer-1"),
            removable("clean and unused")
        );
    }

    #[test]
    fn a_detached_worktree_on_no_ref_stays() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            refs: Vec::new(),
            ..worktree("target/review/1-reviewer-6", 9)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-6"),
            kept("its HEAD s9 is on no ref")
        );
    }

    #[test]
    fn a_head_only_on_a_branch_that_this_run_deletes_is_on_no_ref() {
        let mut s = survey();
        s.branches.push(Branch {
            tree_on_main: Some(true),
            ..branch("squashed", 3)
        });
        s.worktrees.push(Worktree {
            refs: vec!["refs/heads/squashed".to_string()],
            ..worktree("target/review/1-reviewer-6", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-6"),
            kept("its HEAD s2 is on no ref")
        );
    }

    #[test]
    fn a_head_at_the_tip_of_a_branch_that_this_run_deletes_is_on_a_ref() {
        let mut s = survey();
        s.branches.push(Branch {
            tree_on_main: Some(true),
            ..branch("squashed", 3)
        });
        s.worktrees.push(Worktree {
            refs: vec!["refs/heads/squashed".to_string()],
            ..worktree("target/review/1-reviewer-6", 3)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-6"),
            removable("clean and unused")
        );
    }

    #[test]
    fn a_head_on_a_remote_ref_is_on_a_ref() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            refs: vec!["refs/remotes/origin/main".to_string()],
            ..worktree("target/review/1-reviewer-6", 3)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-6"),
            removable("clean and unused")
        );
    }

    #[test]
    fn an_unmerged_agent_worktree_stays() {
        let mut s = survey();
        s.branches.push(branch("agent-y", 2));
        s.worktrees.push(Worktree {
            branch: Some("agent-y".to_string()),
            refs: vec!["refs/heads/agent-y".to_string()],
            ..worktree(".claude/worktrees/agent-y", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, ".claude/worktrees/agent-y"),
            kept("not merged")
        );
        assert_eq!(
            verdict(&s, Kind::Branch, "agent-y"),
            kept("the current branch of a worktree")
        );
    }

    #[test]
    fn an_agent_worktree_on_an_ancestor_of_origin_main_goes() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            merged: Some(true),
            ..worktree(".claude/worktrees/agent-x", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, ".claude/worktrees/agent-x"),
            removable("clean, unused and merged")
        );
    }

    #[test]
    fn an_agent_worktree_on_a_squash_merged_branch_goes() {
        let mut s = survey();
        s.branches.push(Branch {
            tree_on_main: Some(true),
            ..branch("agent-z", 2)
        });
        s.worktrees.push(Worktree {
            branch: Some("agent-z".to_string()),
            refs: vec!["refs/heads/agent-z".to_string()],
            ..worktree(".claude/worktrees/agent-z", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, ".claude/worktrees/agent-z"),
            removable("clean, unused and merged")
        );
        assert!(matches!(
            verdict(&s, Kind::Branch, "agent-z"),
            Verdict::Removable(_)
        ));
    }

    #[test]
    fn a_prunable_worktree_on_a_ref_goes() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            prunable: true,
            status: None,
            ..worktree("target/review/1-reviewer-7", 1)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-7"),
            removable("prunable: its folder is gone")
        );
    }

    #[test]
    fn a_prunable_worktree_on_no_ref_stays_with_advice() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            prunable: true,
            status: None,
            refs: Vec::new(),
            ..worktree("target/review/1-reviewer-7", 9)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-7"),
            kept(
                "prunable, and its HEAD s9 is on no ref: run `git worktree repair`, or prune it by hand"
            )
        );
    }

    #[test]
    fn a_worktree_that_holds_a_worktree_that_stays_stays() {
        let mut s = survey();
        s.worktrees.push(Worktree {
            merged: Some(true),
            ..worktree(".claude/worktrees/agent-x", 2)
        });
        s.worktrees.push(Worktree {
            status: Some(Status {
                changes: true,
                ignored: false,
            }),
            ..worktree(".claude/worktrees/agent-x/target/review/1-reviewer-1", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, ".claude/worktrees/agent-x"),
            kept("holds a worktree that stays")
        );
    }

    #[test]
    fn a_worktree_that_stays_keeps_each_worktree_above_it() {
        let mut s = survey();
        s.worktrees.push(worktree("target/review/1-reviewer-1", 2));
        s.worktrees.push(worktree(
            "target/review/1-reviewer-1/target/review/1-reviewer-2",
            2,
        ));
        s.worktrees.push(Worktree {
            locked: true,
            ..worktree(
                "target/review/1-reviewer-1/target/review/1-reviewer-2/target/review/1-reviewer-3",
                2,
            )
        });
        // A worktree beside another, with a name that starts the same, is
        // not under it.
        s.worktrees.push(worktree("target/review/1-reviewer-10", 2));
        s.worktrees.push(Worktree {
            locked: true,
            ..worktree("target/review/1-reviewer-100", 2)
        });
        for name in [
            "target/review/1-reviewer-1",
            "target/review/1-reviewer-1/target/review/1-reviewer-2",
        ] {
            assert_eq!(
                verdict(&s, Kind::Worktree, name),
                kept("holds a worktree that stays"),
                "{name}"
            );
        }
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-10"),
            removable("clean and unused")
        );
    }

    #[test]
    fn a_worktree_that_holds_a_prunable_worktree_that_stays_stays() {
        let mut s = survey();
        s.worktrees.push(worktree("target/review/1-reviewer-1", 2));
        s.worktrees.push(Worktree {
            prunable: true,
            refs: Vec::new(),
            ..worktree("target/review/1-reviewer-1/sub", 9)
        });
        assert_eq!(
            verdict(&s, Kind::Worktree, "target/review/1-reviewer-1"),
            kept("holds a worktree that stays")
        );
    }

    #[test]
    fn the_branch_of_a_worktree_kept_for_a_nested_worktree_stays() {
        let mut s = survey();
        s.branches.push(Branch {
            ancestor: Some(true),
            ..branch("agent-x", 2)
        });
        s.worktrees.push(Worktree {
            branch: Some("agent-x".to_string()),
            merged: Some(true),
            ..worktree(".claude/worktrees/agent-x", 2)
        });
        s.worktrees.push(Worktree {
            locked: true,
            ..worktree(".claude/worktrees/agent-x/target/review/1-reviewer-1", 2)
        });
        assert_eq!(
            verdict(&s, Kind::Branch, "agent-x"),
            kept("the current branch of a worktree")
        );
    }

    // Run folders.

    #[test]
    fn a_run_folder_with_a_copied_log_goes() {
        let mut s = survey();
        s.run_folders.push(run_folder("old", Log::Copied));
        assert_eq!(
            verdict(&s, Kind::RunFolder, "target/review/run/old"),
            removable("its log.md has a copy in docs/reviews/")
        );
    }

    #[test]
    fn the_run_folder_of_a_current_branch_stays() {
        let mut s = survey();
        s.worktrees[0].branch = Some("harness/phase-3b".to_string());
        s.run_folders
            .push(run_folder("harness-phase-3b", Log::Copied));
        assert_eq!(
            verdict(&s, Kind::RunFolder, "target/review/run/harness-phase-3b"),
            kept("the current branch of a worktree")
        );
    }

    #[test]
    fn the_run_folder_of_a_detached_worktree_that_stays_stays() {
        let mut s = survey();
        s.worktrees[0].branch = None;
        s.run_folders.push(run_folder("detached-s1", Log::Copied));
        assert_eq!(
            verdict(&s, Kind::RunFolder, "target/review/run/detached-s1"),
            kept("the current branch of a worktree")
        );
    }

    #[test]
    fn a_run_folder_with_an_open_round_stays() {
        let mut s = survey();
        s.run_folders.push(RunFolder {
            open_rounds: vec![3],
            ..run_folder("other", Log::Copied)
        });
        assert_eq!(
            verdict(&s, Kind::RunFolder, "target/review/run/other"),
            kept("round 3 is open")
        );
    }

    #[test]
    fn a_run_folder_with_no_log_stays() {
        let mut s = survey();
        s.run_folders.push(run_folder("nolog", Log::Missing));
        assert_eq!(
            verdict(&s, Kind::RunFolder, "target/review/run/nolog"),
            kept("no log.md")
        );
    }

    #[test]
    fn a_run_folder_whose_log_has_no_copy_stays() {
        let mut s = survey();
        s.run_folders.push(run_folder("nolog-copy", Log::NotCopied));
        assert_eq!(
            verdict(&s, Kind::RunFolder, "target/review/run/nolog-copy"),
            kept("log.md has no copy in docs/reviews/")
        );
    }

    #[test]
    fn the_run_folder_of_a_branch_with_an_open_pr_stays() {
        let mut s = survey();
        s.run_folders.push(run_folder("feature-open", Log::Copied));
        s.open_prs = Some(vec!["feature/open".to_string()]);
        assert_eq!(
            verdict(&s, Kind::RunFolder, "target/review/run/feature-open"),
            kept("a branch with an open PR")
        );
    }

    #[test]
    fn a_run_folder_stays_when_the_open_prs_are_unknown() {
        let mut s = survey();
        s.run_folders.push(run_folder("old", Log::Copied));
        s.open_prs = None;
        assert_eq!(
            verdict(&s, Kind::RunFolder, "target/review/run/old"),
            kept("cannot read open PRs")
        );
    }

    // Build output.

    #[test]
    fn build_output_goes() {
        let mut s = survey();
        s.builds.push(Folder {
            path: PathBuf::from("/r/app/dist"),
            size: 3,
        });
        assert_eq!(
            verdict(&s, Kind::Build, "app/dist"),
            removable("build output")
        );
    }

    // Pid files.

    fn pid_survey(pid: Option<u32>, process: Process) -> Survey {
        let mut s = survey();
        s.pid_files.push(PidFile {
            path: PathBuf::from("/r/target/xtask-serve/8080.pid"),
            port: 8080,
            pid,
            process,
        });
        s
    }

    fn pid_verdict_of(pid: Option<u32>, process: Process) -> (Verdict, Option<u32>) {
        let s = pid_survey(pid, process);
        let items = plan(&s);
        let item = items.iter().find(|i| i.kind == Kind::PidFile).unwrap();
        (item.verdict.clone(), item.pid)
    }

    #[test]
    fn a_running_xtask_serve_is_stopped() {
        assert_eq!(
            pid_verdict_of(Some(42), Process::Serve),
            (
                removable("a running xtask serve on port 8080 (pid 42)"),
                Some(42)
            )
        );
    }

    #[test]
    fn a_pid_that_has_ended_is_stale() {
        assert_eq!(
            pid_verdict_of(Some(42), Process::Ended),
            (removable("stale: pid 42 has ended"), None)
        );
    }

    #[test]
    fn a_pid_of_another_program_is_stale_and_not_signalled() {
        assert_eq!(
            pid_verdict_of(Some(42), Process::Other),
            (
                removable("stale: pid 42 runs another program, which stays"),
                None
            )
        );
    }

    #[test]
    fn a_serve_on_another_port_is_stale_and_not_signalled() {
        assert_eq!(
            pid_verdict_of(Some(42), Process::NotListening),
            (
                removable("stale: pid 42 does not listen on port 8080, and stays"),
                None
            )
        );
    }

    #[test]
    fn a_file_with_no_pid_is_stale() {
        assert_eq!(
            pid_verdict_of(None, Process::Ended),
            (removable("stale: it holds no pid"), None)
        );
    }

    #[test]
    fn a_pid_file_stays_when_its_process_is_unknown() {
        assert_eq!(
            pid_verdict_of(
                Some(42),
                Process::Unknown("the state of pid 42".to_string())
            ),
            (kept("cannot read the state of pid 42"), None)
        );
    }

    // Scratch roots.

    fn scratch_survey() -> Survey {
        let mut s = survey();
        s.scratch.push(Folder {
            path: PathBuf::from("/tmp/run-1"),
            size: 7,
        });
        s
    }

    #[test]
    fn a_scratch_root_goes() {
        assert_eq!(
            verdict(&scratch_survey(), Kind::Scratch, "/tmp/run-1"),
            removable("a scratch root named with --also")
        );
    }

    #[test]
    fn a_scratch_root_that_a_process_uses_stays() {
        let mut s = scratch_survey();
        s.open_paths = Some(vec![PathBuf::from("/tmp/run-1/clone/a.txt")]);
        assert_eq!(
            verdict(&s, Kind::Scratch, "/tmp/run-1"),
            kept("a process uses it")
        );
    }

    #[test]
    fn a_scratch_root_stays_when_the_processes_are_unknown() {
        let mut s = scratch_survey();
        s.open_paths = None;
        assert_eq!(
            verdict(&s, Kind::Scratch, "/tmp/run-1"),
            kept("cannot read processes")
        );
    }

    #[test]
    fn a_scratch_root_with_a_worktree_stays() {
        let mut s = scratch_survey();
        s.worktrees.push(Worktree {
            path: PathBuf::from("/tmp/run-1/wt"),
            ..worktree("", 1)
        });
        assert_eq!(
            verdict(&s, Kind::Scratch, "/tmp/run-1"),
            kept("holds a worktree of this repo")
        );
    }

    // The output.

    #[test]
    fn the_report_lists_removable_then_kept_items_with_sizes_and_tips() {
        let mut s = survey();
        s.builds.push(Folder {
            path: PathBuf::from("/r/target/e2e"),
            size: 2048,
        });
        s.branches.push(Branch {
            ancestor: Some(true),
            ..branch("ancestor", 2)
        });
        let lines = report(&plan(&s), 3 << 20, &["/tmp/x".to_string()]);
        assert_eq!(
            lines,
            [
                "can remove (2 items, 2.0 KiB):",
                "  branch ancestor (s2): its tip is an ancestor of origin/main",
                "  build target/e2e (2.0 KiB): build output",
                "keeps (1 items):",
                "  branch main (s1): main and gh-pages stay",
                "target/ holds 3.0 MiB.  `cargo clean` removes it: run it by hand.",
                "run `cargo run -p xtask -- clean --yes --also /tmp/x` to remove the items above.",
            ]
        );
    }

    #[test]
    fn with_nothing_removable_the_report_says_so_and_gives_no_command() {
        let lines = report(&plan(&survey()), 0, &[]);
        assert_eq!(
            lines,
            [
                "nothing to remove",
                "keeps (1 items):",
                "  branch main (s1): main and gh-pages stay",
                "target/ holds 0 B.  `cargo clean` removes it: run it by hand.",
            ]
        );
    }

    #[test]
    fn sizes_are_in_binary_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(3 << 19), "1.5 MiB");
        assert_eq!(human_size(5 << 30), "5.0 GiB");
    }

    #[test]
    fn the_command_line_takes_yes_and_also() {
        let args =
            |words: &[&str]| -> Vec<String> { words.iter().map(|w| w.to_string()).collect() };
        assert_eq!(parse_args(&[]), Some(Options::default()));
        assert_eq!(
            parse_args(&args(&["--also", "/tmp/a", "--yes", "--also", "/tmp/b"])),
            Some(Options {
                yes: true,
                also: vec!["/tmp/a".to_string(), "/tmp/b".to_string()],
            })
        );
        assert_eq!(parse_args(&args(&["--also"])), None);
        assert_eq!(parse_args(&args(&["now"])), None);
    }

    #[test]
    fn only_the_xtask_binary_with_serve_runs_serve() {
        assert!(runs_serve(
            "/r/target/debug/xtask serve app/dist --port 8080"
        ));
        assert!(runs_serve("xtask serve"));
        assert!(!runs_serve("cargo run -p xtask -- serve app/dist"));
        assert!(!runs_serve("/r/target/debug/xtask-serve x"));
        assert!(!runs_serve("/r/target/debug/xtask e2e serve"));
        assert!(!runs_serve("sleep 300"));
    }

    #[test]
    fn the_worktree_listing_is_read_with_its_marks() {
        let listing = "worktree /r\0HEAD 1\0branch refs/heads/main\0\0\
            worktree /bare\0bare\0\0\
            worktree /nowhere/a/b\0HEAD 2\0detached\0locked why\0prunable gone\0\0";
        let worktrees = parse_worktrees(listing);
        assert_eq!(worktrees.len(), 2, "{worktrees:?}");
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert!(!worktrees[0].locked && !worktrees[0].prunable);
        assert_eq!(worktrees[1].path, PathBuf::from("/nowhere/a/b"));
        assert_eq!(worktrees[1].head, "2");
        assert_eq!(worktrees[1].branch, None);
        assert!(worktrees[1].locked && worktrees[1].prunable);
    }

    #[test]
    fn a_path_that_is_gone_is_canonical_from_its_nearest_folder() {
        let temp = std::env::temp_dir();
        let gone = temp.join("xtask-clean-no-such-folder/a");
        assert_eq!(
            canonical(&gone),
            temp.canonicalize()
                .unwrap()
                .join("xtask-clean-no-such-folder/a")
        );
    }

    // `--yes`.

    #[test]
    fn the_report_ends_with_the_yes_command_and_each_also() {
        let mut s = survey();
        s.builds.push(Folder {
            path: PathBuf::from("/r/target/e2e"),
            size: 1,
        });
        let lines = report(&plan(&s), 0, &["/tmp/a".to_string(), "/tmp/b".to_string()]);
        assert_eq!(
            lines.last().map(String::as_str),
            Some(
                "run `cargo run -p xtask -- clean --yes --also /tmp/a --also /tmp/b` to remove the items above."
            )
        );
        let lines = report(&plan(&survey()), 0, &["/tmp/a".to_string()]);
        assert!(!lines.iter().any(|l| l.starts_with("run `")), "{lines:?}");
    }

    #[test]
    fn the_command_line_takes_yes() {
        assert_eq!(
            parse_args(&["--yes".to_string()]),
            Some(Options {
                yes: true,
                also: Vec::new(),
            })
        );
    }

    /// A child, stopped and reaped when the test ends.
    struct Child(std::process::Child);

    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Reaps `child` in a thread, as a shell would, so `stop` sees it end.
    fn reaped(child: std::process::Child) -> (u32, std::thread::JoinHandle<()>) {
        let pid = child.id();
        let mut guard = Child(child);
        let handle = std::thread::spawn(move || {
            let _ = guard.0.wait();
        });
        (pid, handle)
    }

    #[test]
    fn stop_ends_a_process_with_sigterm() {
        let child = Command::new("sleep").arg("300").spawn().unwrap();
        let (pid, handle) = reaped(child);
        let start = Instant::now();
        assert_eq!(stop(pid), Ok(()));
        assert!(start.elapsed() < STOP_WAIT, "{:?}", start.elapsed());
        handle.join().unwrap();
    }

    #[test]
    fn stop_fails_on_a_process_that_ignores_sigterm() {
        let child = Command::new("sh")
            .args(["-c", "trap '' TERM; exec sleep 300"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let guard = Child(child);
        // Wait for the shell to set the trap and become `sleep`.
        let until = Instant::now() + Duration::from_secs(5);
        while !matches!(
            ask(Command::new("ps").args(["-p", &pid.to_string(), "-o", "command="])),
            Answer::Yes(command) if command.starts_with("sleep")
        ) {
            assert!(Instant::now() < until, "pid {pid} is no sleep");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(stop(pid), Err(format!("pid {pid} still runs after 5 s")));
        drop(guard);
    }

    #[test]
    fn a_worktree_is_removed_without_force() {
        // With `--force`, git would remove a worktree that changed after the
        // second survey.
        assert_eq!(WORKTREE_REMOVE, ["worktree", "remove"]);
    }

    #[test]
    fn the_largest_unit_is_tib() {
        assert_eq!(human_size(1 << 50), "1024.0 TiB");
        assert_eq!(human_size(1 << 40), "1.0 TiB");
    }

    #[test]
    fn stop_of_a_pid_that_has_ended_is_ok() {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert_eq!(stop(pid), Ok(()));
    }

    #[test]
    fn ask_reads_the_exit_code_stdout_and_stderr() {
        let sh = |script: &str| ask(Command::new("sh").args(["-c", script]));
        assert_eq!(sh("echo hi"), Answer::Yes("hi".to_string()));
        assert_eq!(sh("exit 0"), Answer::No);
        assert_eq!(sh("exit 1"), Answer::No);
        assert_eq!(sh("exit 2"), Answer::Unknown);
        assert_eq!(sh("echo hi; exit 1"), Answer::Unknown);
        assert_eq!(sh("echo oops >&2; exit 1"), Answer::Unknown);
        assert_eq!(sh("echo oops >&2"), Answer::Unknown);
        assert_eq!(ask(&mut Command::new("/no/such/program")), Answer::Unknown);
    }

    #[test]
    fn lsof_gives_no_warnings() {
        // Linux's `lsof` warns on stderr, which `ask` reads as unknown.
        assert!(LSOF_OPEN.contains(&"-w"), "{LSOF_OPEN:?}");
        let args = lsof_listen_args(42, 8080);
        assert!(args.iter().any(|a| a == "-w"), "{args:?}");
        // Without `-a`, `lsof` gives the pid if it has any TCP port, or if
        // anything listens on the port.
        assert!(args.iter().any(|a| a == "-a"), "{args:?}");
        assert!(args.iter().any(|a| a == "-sTCP:LISTEN"), "{args:?}");
        assert!(args.iter().any(|a| a == "-iTCP:8080"), "{args:?}");
        assert!(
            args.windows(2).any(|w| w[0] == "-p" && w[1] == "42"),
            "{args:?}"
        );
    }

    #[test]
    fn dir_size_adds_the_files_of_a_tree() {
        let dir = std::env::temp_dir().join(format!("xtask-clean-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/x"), "abc").unwrap();
        std::fs::write(dir.join("y"), "defghi").unwrap();
        std::os::unix::fs::symlink("/no/such/target/at/all", dir.join("link")).unwrap();
        let size = dir_size(&dir);
        let link = dir.join("link").symlink_metadata().unwrap().len();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(size, 9 + link);
    }

    #[test]
    fn the_total_counts_a_nested_removable_worktree_once() {
        let mut s = survey();
        s.worktrees.push(worktree("target/review/1-reviewer-1", 1));
        s.worktrees.push(worktree(
            "target/review/1-reviewer-1/target/review/1-reviewer-2",
            1,
        ));
        s.worktrees.push(worktree("target/review/1-reviewer-10", 1));
        let lines = report(&plan(&s), 0, &[]);
        // Each worktree has 10 bytes, and the nested one is in the 10 of the
        // worktree that holds it.
        assert_eq!(lines[0], "can remove (3 items, 20 B):", "{lines:#?}");
    }
}
