mod common;

use std::{fs, path::Path};

use common::rel;
use yusra_index::{PathFilter, walk};

fn write(root: &Path, path: &str, content: &str) {
    let full = root.join(path);
    if let Some(parent) = full.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        panic!("{e}");
    }
    if let Err(e) = fs::write(&full, content) {
        panic!("{e}");
    }
}

const TREE: &[(&str, &str)] = &[
    (
        ".gitignore",
        "target/\n*.log\n!keep.log\nbuild\n/root-only.txt\n",
    ),
    (".git/info/exclude", "local-notes.md\n"),
    (".git/HEAD", "ref: refs/heads/main\n"),
    (".yusra/state.db", "x"),
    (".yusraignore", "secret/\n"),
    ("src/main.rs", "x"),
    ("src/app.log", "x"),
    ("src/keep.log", "x"),
    ("src/build/out.rs", "x"),
    ("src/root-only.txt", "x"),
    ("root-only.txt", "x"),
    ("local-notes.md", "x"),
    ("target/debug/a.rs", "x"),
    ("secret/key.rs", "x"),
    ("dist/bundle.js", "x"),
    ("web/.gitignore", "*.gen.ts\n!important.gen.ts\n"),
    ("web/a.gen.ts", "x"),
    ("web/important.gen.ts", "x"),
    ("web/app.ts", "x"),
    ("web/sub/.yusraignore", "fixtures/\n"),
    ("web/sub/fixtures/f.json", "x"),
    ("web/sub/ok.ts", "x"),
    ("vendor/lib/.git/HEAD", "ref: refs/heads/main\n"),
    ("vendor/lib/.gitignore", "!*.log\n"),
    ("vendor/lib/trace.log", "x"),
    ("vendor/lib/a.go", "x"),
    ("docs/.yusra/note.md", "x"),
];

#[test]
fn filter_agrees_with_walk() {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    let root = dir.path();
    for (path, content) in TREE {
        write(root, path, content);
    }
    let extra = vec!["dist/".to_owned()];
    let walked: Vec<String> = match walk(root, &extra) {
        Ok(w) => w.files.iter().map(|f| f.path.to_string()).collect(),
        Err(e) => panic!("{e}"),
    };
    let mut filter = match PathFilter::new(root, &extra) {
        Ok(f) => f,
        Err(e) => panic!("{e}"),
    };
    for (path, _) in TREE {
        let in_walk = walked.iter().any(|w| w == path);
        let ignored = filter.is_ignored(&rel(path), false);
        assert_eq!(
            in_walk, !ignored,
            "{path}: walk={in_walk}, filter ignored={ignored}"
        );
    }
    // Contoh eksplisit agar maksud aturannya terbaca.
    assert!(walked.contains(&"src/keep.log".to_owned()));
    assert!(walked.contains(&"web/important.gen.ts".to_owned()));
    assert!(walked.contains(&"vendor/lib/trace.log".to_owned()));
    // `/root-only.txt` berjangkar ke root `.gitignore`.
    assert!(!walked.contains(&"root-only.txt".to_owned()));
    assert!(walked.contains(&"src/root-only.txt".to_owned()));
    assert!(filter.is_ignored(&rel("target"), true));
    assert!(filter.is_ignored(&rel("target/new/file.rs"), false));
    assert!(filter.is_ignored(&rel(".git/index"), false));
    assert!(!filter.is_ignored(&rel("src/new.rs"), false));
}

#[test]
fn reload_picks_up_changed_rules() {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    let root = dir.path();
    write(root, ".gitignore", "*.tmp\n");
    let mut filter = match PathFilter::new(root, &[]) {
        Ok(f) => f,
        Err(e) => panic!("{e}"),
    };
    assert!(filter.is_ignored(&rel("a.tmp"), false));
    assert!(!filter.is_ignored(&rel("a.bak"), false));

    write(root, ".gitignore", "*.bak\n");
    if let Err(e) = filter.reload() {
        panic!("{e}");
    }
    assert!(!filter.is_ignored(&rel("a.tmp"), false));
    assert!(filter.is_ignored(&rel("a.bak"), false));
}
