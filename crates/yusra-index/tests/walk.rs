mod common;

use std::{fs, path::Path};

use common::rel;
use yusra_core::YusraError;
use yusra_index::{builtin_adapters, index_file, walk};

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

fn temp() -> tempfile::TempDir {
    match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    }
}

fn walked(root: &Path, extra_ignore: &[&str]) -> Vec<String> {
    let extra: Vec<String> = extra_ignore.iter().map(|s| (*s).to_owned()).collect();
    match walk(root, &extra) {
        Ok(w) => w.files.iter().map(|f| f.path.to_string()).collect(),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn respects_ignore_files_and_skips_state_dirs() {
    let dir = temp();
    let root = dir.path();
    write(root, ".gitignore", "target/\n*.log\n");
    write(root, ".yusraignore", "secret/\n");
    write(root, "src/main.rs", "fn main() {}\n");
    write(root, "src/lib.rs", "pub fn a() {}\n");
    write(root, "src/nested/mod.rs", "\n");
    write(root, "target/debug/out.rs", "\n");
    write(root, "app.log", "log\n");
    write(root, "secret/key.rs", "\n");
    write(root, "dist/bundle.js", "\n");
    write(root, "web/dist/x.js", "\n");
    write(root, ".git/HEAD", "ref: refs/heads/main\n");
    write(root, ".yusra/state.db", "\n");
    write(root, "vendor/lib/.git", "gitdir: ../.git/modules/lib\n");
    write(root, "vendor/lib/a.go", "package lib\n");
    write(root, "docs/.yusra/keep.md", "\n");
    write(root, ".github/workflows/ci.yml", "name: ci\n");
    write(root, ".env", "X=1\n");

    assert_eq!(
        walked(root, &["dist/"]),
        [
            ".env",
            ".github/workflows/ci.yml",
            ".gitignore",
            ".yusraignore",
            "docs/.yusra/keep.md",
            "src/lib.rs",
            "src/main.rs",
            "src/nested/mod.rs",
            "vendor/lib/a.go",
        ]
    );
}

#[test]
fn ignore_files_apply_without_git_dir() {
    let dir = temp();
    let root = dir.path();
    write(root, ".gitignore", "build/\n");
    write(root, "build/a.rs", "\n");
    write(root, "src/a.rs", "\n");
    assert_eq!(walked(root, &[]), [".gitignore", "src/a.rs"]);
}

#[test]
fn reports_sizes() {
    let dir = temp();
    write(dir.path(), "a.txt", "12345");
    let w = match walk(dir.path(), &[]) {
        Ok(w) => w,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(w.files.len(), 1);
    assert_eq!(w.files[0].size, 5);
}

#[test]
fn rejects_negated_extra_ignore() {
    let dir = temp();
    assert!(walk(dir.path(), &["!keep.rs".to_owned()]).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_are_skipped() {
    use yusra_index::Skipped;

    let dir = temp();
    let outside = temp();
    let root = dir.path();
    write(outside.path(), "passwd", "root:x:0:0\n");
    write(root, "src/a.rs", "\n");
    let link = |target: &Path, name: &str| {
        if let Err(e) = std::os::unix::fs::symlink(target, root.join(name)) {
            panic!("{e}");
        }
    };
    link(&outside.path().join("passwd"), "leak");
    link(outside.path(), "leakdir");

    let w = match walk(root, &[]) {
        Ok(w) => w,
        Err(e) => panic!("{e}"),
    };
    let files: Vec<String> = w.files.iter().map(|f| f.path.to_string()).collect();
    assert_eq!(files, ["src/a.rs"]);
    assert!(w.skipped.contains(&Skipped::Symlink("leak".to_owned())));
    assert!(w.skipped.contains(&Skipped::Symlink("leakdir".to_owned())));

    let adapters = match builtin_adapters() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    };
    assert!(matches!(
        index_file(root, &rel("leak"), &adapters, 1 << 20),
        Err(YusraError::NotFound { .. })
    ));
}

#[cfg(feature = "lang-rust")]
#[test]
fn index_file_hashes_and_parses() {
    let dir = temp();
    let root = dir.path();
    let src = "/// Doc.\npub fn hello() -> u8 {\n    1\n}\n";
    write(root, "src/lib.rs", src);
    write(root, "README.md", "# halo\n");
    let adapters = match builtin_adapters() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    };

    let rust = match index_file(root, &rel("src/lib.rs"), &adapters, 1 << 20) {
        Ok(f) => f,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(rust.hash, yusra_index::hash_bytes(src.as_bytes()));
    assert_eq!(rust.language, Some("rust"));
    let Some(parsed) = rust.parsed else {
        panic!("file Rust harus diparse");
    };
    assert!(!parsed.has_errors);
    let names: Vec<&str> = parsed
        .symbols
        .iter()
        .map(|s| s.symbol_path.as_str())
        .collect();
    assert_eq!(names, ["hello"]);

    let md = match index_file(root, &rel("README.md"), &adapters, 1 << 20) {
        Ok(f) => f,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(md.language, None);
    assert!(md.parsed.is_none());
}

#[cfg(feature = "lang-rust")]
#[test]
fn large_files_are_hashed_but_not_parsed() {
    let dir = temp();
    let src = "pub fn a() {}\n";
    write(dir.path(), "big.rs", src);
    let adapters = match builtin_adapters() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    };
    let f = match index_file(dir.path(), &rel("big.rs"), &adapters, 4) {
        Ok(f) => f,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(f.hash, yusra_index::hash_bytes(src.as_bytes()));
    assert_eq!(f.language, Some("rust"));
    assert!(f.parsed.is_none());
}

#[test]
fn index_file_missing_is_not_found() {
    let dir = temp();
    let adapters = match builtin_adapters() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    };
    assert!(matches!(
        index_file(dir.path(), &rel("nope.rs"), &adapters, 1 << 20),
        Err(YusraError::NotFound { .. })
    ));
}
