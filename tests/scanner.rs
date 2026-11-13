use find_dev_data::{
    export,
    rules::Category,
    scanner::{self, Event, Finding, ScanOptions},
};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tempfile::TempDir;

fn put(root: &Path, path: &str, bytes: usize) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![42; bytes]).unwrap();
}

fn run(root: &Path, home: Option<&Path>) -> Vec<Finding> {
    let mut findings = Vec::new();
    scanner::scan(
        ScanOptions {
            root: root.into(),
            home: home.map(Path::to_path_buf),
            include_volumes: false,
        },
        Arc::new(AtomicBool::new(false)),
        |event| {
            if let Event::Found(finding) = event {
                scanner::merge_finding(&mut findings, finding);
            }
        },
    )
    .unwrap();
    findings
}

#[test]
fn finds_developer_data_and_models_without_double_counting_nested_matches() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    put(root, "web/package.json", 2);
    put(root, "web/node_modules/a/index.js", 100);
    put(root, "web/node_modules/a/node_modules/b/index.js", 200);
    put(root, "rust/Cargo.toml", 1);
    put(root, "rust/target/debug/program", 400);
    put(root, "swift/Package.swift", 1);
    put(root, "swift/.build/debug/program", 800);
    put(root, "python/.venv/pyvenv.cfg", 10);
    put(root, "python/.venv/lib/weights.safetensors", 1600);
    put(
        root,
        "Library/Developer/Xcode/DerivedData/First-abcd/Build/foo",
        3200,
    );
    put(
        root,
        "Library/Developer/Xcode/DerivedData/Second-efgh/info.plist",
        32,
    );
    put(root, ".cargo/registry/src/crate/target/.rustc_info.json", 1);
    put(root, ".cargo/registry/cache/crate.tar", 50);
    put(root, ".ollama/models/blobs/sha256-a", 6400);
    put(root, ".lmstudio/models/publisher/model/model.gguf", 12800);
    put(root, ".omlx/models/test/model.safetensors", 25600);
    put(root, ".cache/huggingface/hub/models--a/blobs/hash", 51200);
    put(root, "custom/converted/model.safetensors", 102400);
    let findings = run(root, Some(root));
    assert_eq!(
        findings.len(),
        12,
        "{:#?}",
        findings.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.tool == "Xcode DerivedData")
            .count(),
        2
    );
    let nodes = findings.iter().find(|f| f.tool == "Node.js").unwrap();
    assert_eq!((nodes.logical_bytes, nodes.files), (300, 2));
    assert!(findings.iter().all(|f| f.complete));
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.category == Category::Models)
            .count(),
        5
    );
    for a in &findings {
        for b in &findings {
            assert!(a.path == b.path || !a.path.starts_with(&b.path));
        }
    }
}

#[test]
fn does_not_guess_from_ambiguous_folder_names() {
    let tmp = TempDir::new().unwrap();
    for path in [
        "archery/target/notes",
        "documents/Build/notes",
        "app/.build/notes",
        "code/venv/notes",
        "code/env/notes",
    ] {
        put(tmp.path(), path, 10);
    }
    assert!(run(tmp.path(), None).is_empty());
    put(tmp.path(), "app/App.xcodeproj/project.pbxproj", 1);
    put(tmp.path(), "app/Build/a.o", 25);
    put(tmp.path(), "cmake/build/CMakeCache.txt", 5);
    assert_eq!(run(tmp.path(), None).len(), 2);
}

#[test]
fn never_visits_known_locations_outside_the_selected_root() {
    let tmp = TempDir::new().unwrap();
    put(tmp.path(), ".ollama/models/blobs/a", 100);
    fs::create_dir(tmp.path().join("project")).unwrap();
    assert!(run(&tmp.path().join("project"), Some(tmp.path())).is_empty());
}

#[cfg(unix)]
#[test]
fn symlinks_do_not_escape_the_root_or_create_loops_and_hardlinks_count_once() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    put(tmp.path(), "node_modules/a", 1234);
    put(outside.path(), "big", 10000);
    fs::hard_link(
        tmp.path().join("node_modules/a"),
        tmp.path().join("node_modules/b"),
    )
    .unwrap();
    symlink(outside.path(), tmp.path().join("node_modules/outside")).unwrap();
    symlink(tmp.path(), tmp.path().join("loop")).unwrap();
    let findings = run(tmp.path(), None);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].logical_bytes, 1234);
    assert_eq!(findings[0].files, 1);
    assert_eq!(findings[0].symlinks_skipped, 1);
}

#[test]
fn cancellation_finishes_and_marks_an_interrupted_measurement_partial() {
    let tmp = TempDir::new().unwrap();
    put(tmp.path(), "node_modules/a", 100);
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let mut partial = false;
    let mut finished = false;
    scanner::scan(
        ScanOptions {
            root: tmp.path().into(),
            home: None,
            include_volumes: false,
        },
        cancel,
        |event| match event {
            Event::Progress(p) if p.measuring => signal.store(true, Ordering::Relaxed),
            Event::Found(f) => partial = !f.complete,
            Event::Finished { cancelled, .. } => finished = cancelled,
            _ => {}
        },
    )
    .unwrap();
    assert!(partial && finished);
}

#[test]
fn reports_unreadable_entries_and_marks_their_finding_partial() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "node_modules/closed/data", 10);
        let closed = tmp.path().join("node_modules/closed");
        fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
        // Root can bypass permissions. Skip the assertion in that environment.
        let unreadable = fs::read_dir(&closed).is_err();
        let findings = run(tmp.path(), None);
        fs::set_permissions(&closed, fs::Permissions::from_mode(0o700)).unwrap();
        if unreadable {
            assert!(!findings[0].complete);
            assert_eq!(findings[0].errors, 1);
        }
    }
}

#[test]
fn csv_escapes_paths_and_records_partial_state() {
    let tmp = TempDir::new().unwrap();
    put(tmp.path(), "a,\"quoted\"/node_modules/a", 7);
    let findings = run(tmp.path(), None);
    let mut csv = Vec::new();
    export::write_csv(&mut csv, &findings.iter().collect::<Vec<_>>()).unwrap();
    let csv = String::from_utf8(csv).unwrap();
    assert!(csv.contains("a,\"\"quoted\"\""));
    assert!(csv.contains(",7,"));
    assert!(csv.contains(",true,0,0"));
}

#[test]
fn validates_root_and_resolves_a_selected_symlink() {
    let tmp = TempDir::new().unwrap();
    let options = ScanOptions {
        root: tmp.path().join("missing"),
        home: None,
        include_volumes: false,
    };
    assert!(scanner::scan(options, Arc::new(AtomicBool::new(false)), |_| {}).is_err());
    #[cfg(unix)]
    {
        put(tmp.path(), "real/node_modules/file", 20);
        std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("alias")).unwrap();
        assert_eq!(run(&tmp.path().join("alias"), None).len(), 1);
    }
}

#[test]
fn outer_discovery_replaces_a_fast_path_child() {
    let tmp = TempDir::new().unwrap();
    put(tmp.path(), "node_modules/model.gguf", 20);
    put(tmp.path(), "node_modules/.ollama/models/blobs/a", 30);
    let findings = run(tmp.path(), Some(&tmp.path().join("node_modules")));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].tool, "Node.js");
    assert_eq!(findings[0].logical_bytes, 50);
}
