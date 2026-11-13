use crate::rules::{self, Category, Rule};
use serde::Serialize;
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use walkdir::WalkDir;

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub path: PathBuf,
    pub tool: String,
    pub category: Category,
    pub detail: String,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub files: u64,
    pub symlinks_skipped: u64,
    pub errors: u64,
    pub complete: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub directories: u64,
    pub files_measured: u64,
    pub errors: u64,
    pub current: PathBuf,
    pub measuring: bool,
    pub elapsed: Duration,
}

#[derive(Clone, Debug)]
pub struct ScanError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug)]
pub enum Event {
    Found(Finding),
    Progress(Progress),
    Warning(ScanError),
    Finished { cancelled: bool, progress: Progress },
    Failed(String),
}

#[derive(Clone)]
pub struct ScanOptions {
    pub root: PathBuf,
    pub home: Option<PathBuf>,
    pub include_volumes: bool,
}

pub fn scan(
    options: ScanOptions,
    cancel: Arc<AtomicBool>,
    mut emit: impl FnMut(Event),
) -> io::Result<()> {
    let root = fs::canonicalize(&options.root)?;
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose a directory to scan",
        ));
    }
    let mut scanner = Scanner {
        root,
        options,
        cancel,
        emit: &mut emit,
        covered: HashSet::new(),
        progress: Progress::default(),
        start: Instant::now(),
        last_update: Instant::now(),
    };
    scanner.run();
    Ok(())
}

struct Scanner<'a, F: FnMut(Event)> {
    root: PathBuf,
    options: ScanOptions,
    cancel: Arc<AtomicBool>,
    emit: &'a mut F,
    covered: HashSet<PathBuf>,
    progress: Progress,
    start: Instant,
    last_update: Instant,
}

impl<F: FnMut(Event)> Scanner<'_, F> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn run(&mut self) {
        if let Some(home) = &self.options.home {
            let mut known = rules::known_locations(home);
            // Process outer stores first; nested rows must never inflate totals.
            known.sort_by_key(|(path, _)| path.components().count());
            for (path, rule) in known {
                if self.cancelled() {
                    break;
                }
                if let Ok(path) = fs::canonicalize(path)
                    && path.starts_with(&self.root)
                    && path.is_dir()
                    && !self.excluded(&path)
                    && !self.is_covered(&path)
                {
                    self.record(path, rule);
                }
            }
        }

        let mut pending = vec![self.root.clone()];
        while let Some(path) = pending.pop() {
            if self.cancelled() {
                break;
            }
            if self.excluded(&path) || self.is_covered(&path) {
                continue;
            }
            self.progress.directories += 1;
            self.progress.current = path.clone();
            self.progress.measuring = false;
            self.publish_progress(false);
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) => {
                    self.warning(&path, error);
                    continue;
                }
            };
            let mut names = Vec::new();
            let mut children = Vec::new();
            for entry in entries {
                if self.cancelled() {
                    break;
                }
                match entry {
                    Ok(entry) => {
                        match entry.file_type() {
                            Ok(kind) => {
                                // Symlinks are never traversed. Known top-level aliases were canonicalized above.
                                if kind.is_dir() {
                                    children.push(entry.path());
                                }
                                if !kind.is_symlink() {
                                    names.push(entry.file_name());
                                }
                            }
                            Err(error) => self.warning(&entry.path(), error),
                        }
                    }
                    Err(error) => self.warning(&path, error),
                }
            }
            if self.cancelled() {
                break;
            }
            if let Some(rule) = rules::classify(&path, &names) {
                // An outer directory discovered after a fast-path child replaces that child.
                self.record(path, rule);
            } else {
                // Reverse alphabetical order on the stack gives deterministic discovery.
                children.sort_unstable_by(|a, b| b.cmp(a));
                pending.extend(children);
            }
        }
        self.publish_progress(true);
        let cancelled = self.cancelled();
        (self.emit)(Event::Finished {
            cancelled,
            progress: self.progress.clone(),
        });
    }

    fn is_covered(&self, path: &Path) -> bool {
        path.ancestors()
            .any(|ancestor| self.covered.contains(ancestor))
    }

    fn excluded(&self, path: &Path) -> bool {
        if path == self.root {
            return false;
        }
        if !self.options.include_volumes
            && !self.root.starts_with("/Volumes")
            && path.starts_with("/Volumes")
        {
            return true;
        }
        // macOS exposes its Data volume at / as well as /System/Volumes/Data.
        // Virtual filesystems and APFS aliases must not be visited a second time.
        [
            "/dev",
            "/proc",
            "/sys",
            "/net",
            "/Network",
            "/.vol",
            "/System/Volumes",
        ]
        .iter()
        .any(|p| path.starts_with(p))
            || path.file_name().is_some_and(|n| {
                n == ".git"
                    || n == ".Spotlight-V100"
                    || n == ".fseventsd"
                    || n == ".Trashes"
                    || n == ".TimeMachine"
            })
    }

    fn record(&mut self, path: PathBuf, rule: Rule) {
        self.progress.current = path.clone();
        self.progress.measuring = true;
        self.publish_progress(true);
        let mut finding = Finding {
            path: path.clone(),
            tool: rule.tool.into(),
            category: rule.category,
            detail: rule.detail.into(),
            logical_bytes: 0,
            allocated_bytes: 0,
            files: 0,
            symlinks_skipped: 0,
            errors: 0,
            complete: true,
        };
        let mut hardlinks = HashSet::new();
        for entry in WalkDir::new(&path)
            .follow_links(false)
            .follow_root_links(false)
        {
            if self.cancelled() {
                finding.complete = false;
                break;
            }
            match entry {
                Ok(entry) => {
                    if entry.file_type().is_symlink() {
                        finding.symlinks_skipped += 1;
                        continue;
                    }
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    match entry.metadata() {
                        Ok(meta) => {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::MetadataExt;
                                // Only multiply-linked files need tracking; caches may contain millions of files.
                                if meta.nlink() > 1 && !hardlinks.insert((meta.dev(), meta.ino())) {
                                    continue;
                                }
                                finding.allocated_bytes = finding
                                    .allocated_bytes
                                    .saturating_add(meta.blocks().saturating_mul(512));
                            }
                            #[cfg(not(unix))]
                            {
                                finding.allocated_bytes =
                                    finding.allocated_bytes.saturating_add(meta.len());
                            }
                            finding.logical_bytes =
                                finding.logical_bytes.saturating_add(meta.len());
                            finding.files += 1;
                            self.progress.files_measured += 1;
                        }
                        Err(error) => {
                            finding.errors += 1;
                            self.warning(entry.path(), error);
                        }
                    }
                }
                Err(error) => {
                    finding.errors += 1;
                    let error_path = error.path().unwrap_or(&path).to_path_buf();
                    self.warning(&error_path, error);
                }
            }
            self.publish_progress(false);
        }
        finding.complete &= finding.errors == 0;
        self.covered.retain(|p| !p.starts_with(&path));
        self.covered.insert(path);
        (self.emit)(Event::Found(finding));
    }

    fn warning(&mut self, path: &Path, error: impl std::fmt::Display) {
        self.progress.errors += 1;
        // Bound diagnostic memory / channel traffic without hiding the total error count.
        if self.progress.errors <= 200 {
            (self.emit)(Event::Warning(ScanError {
                path: path.to_path_buf(),
                message: error.to_string(),
            }));
        }
    }

    fn publish_progress(&mut self, force: bool) {
        if force || self.last_update.elapsed() >= Duration::from_millis(120) {
            self.progress.elapsed = self.start.elapsed();
            (self.emit)(Event::Progress(self.progress.clone()));
            self.last_update = Instant::now();
        }
    }
}

/// Keep findings disjoint, including when a fast-path child precedes an outer store.
pub fn merge_finding(findings: &mut Vec<Finding>, finding: Finding) {
    if findings
        .iter()
        .any(|f| finding.path != f.path && finding.path.starts_with(&f.path))
    {
        return;
    }
    findings.retain(|f| !f.path.starts_with(&finding.path));
    findings.push(finding);
}
