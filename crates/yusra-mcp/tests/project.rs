use std::{fs, path::Path};

use yusra_core::{ErrorCode, RelPath, YusraError};
use yusra_index::Change;
use yusra_mcp::{
    EstimateTokensRequest, GetProjectMapRequest, ModelFamilyParam, Project, ReadFileRequest,
    ReadSymbolRequest, STATE_DIR, SearchSymbolsRequest, Settings, SymbolKindParam,
};

const AUTH_RS: &str = "\
use crate::token::issue_token;

/// Session pengguna.
pub struct Session {
    pub id: u64,
}

impl Session {
    /// Perbarui token.
    pub fn refresh(&mut self) -> String {
        issue_token(self.id)
    }
}
";

const TOKEN_RS: &str = "\
pub fn issue_token(id: u64) -> String {
    format!(\"t{id}\")
}
";

fn write(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        panic!("{e}");
    }
    if let Err(e) = fs::write(&full, content) {
        panic!("{e}");
    }
}

fn ok<T>(r: Result<T, YusraError>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

fn code<T: std::fmt::Debug>(r: Result<T, YusraError>) -> ErrorCode {
    match r {
        Ok(v) => panic!("seharusnya gagal, dapat {v:?}"),
        Err(e) => e.code(),
    }
}

fn setup() -> (tempfile::TempDir, Project) {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    let root = dir.path();
    write(root, "src/auth.rs", AUTH_RS);
    write(root, "src/token.rs", TOKEN_RS);
    write(root, ".env", "SECRET=1\n");
    write(root, "config/secrets.py", "API_KEY = 'x'\n");
    write(root, ".git/config", "[core]\n");
    if let Err(e) = fs::create_dir(root.join(STATE_DIR)) {
        panic!("{e}");
    }
    let settings = Settings {
        deny_read: [".env", "config/secrets.py"].map(str::to_owned).to_vec(),
        ..Settings::default()
    };
    let project = ok(Project::open(root, settings));
    ok(project.full_index());
    (dir, project)
}

fn read(path: &str) -> ReadFileRequest {
    ReadFileRequest {
        path: path.to_owned(),
        session_id: None,
        with_deps: false,
        max_tokens: None,
    }
}

#[test]
fn open_requires_init() {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    match Project::open(dir.path(), Settings::default()) {
        Ok(_) => panic!("harus gagal tanpa .yusra"),
        Err(e) => assert!(e.to_string().contains("yusrad init"), "{e}"),
    }
}

#[test]
fn full_index_skips_denied_and_is_incremental() {
    let (_dir, project) = setup();
    let hashes = ok(project.store().file_hashes());
    let mut paths: Vec<&str> = hashes.keys().map(RelPath::as_str).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["src/auth.rs", "src/token.rs"]);

    let again = ok(project.full_index());
    assert_eq!(again.files, 2);
    assert_eq!(again.updated, 0);
    assert_eq!(again.removed, 0);
}

#[test]
fn search_and_read_symbol() {
    let (_dir, project) = setup();
    let hits = ok(project.search_symbols(&SearchSymbolsRequest {
        query: "refresh".to_owned(),
        kind: Some(SymbolKindParam::Method),
        limit: None,
    }));
    assert_eq!(hits.symbols.len(), 1);
    let hit = &hits.symbols[0];
    assert_eq!(hit.path, "src/auth.rs");
    assert_eq!(hit.symbol_path, "Session::refresh");

    let sym = ok(project.read_symbol(&ReadSymbolRequest {
        path: "src/auth.rs".to_owned(),
        symbol_path: "Session::refresh".to_owned(),
        session_id: None,
    }));
    // Rentang dimulai di doc comment, tanpa indentasi baris pertama.
    assert!(
        sym.source.starts_with("/// Perbarui token."),
        "{}",
        sym.source
    );
    assert!(sym.source.trim_end().ends_with('}'));
    assert_eq!((sym.start_line, sym.end_line), (9, 12));
    assert_eq!(sym.hash, hit.hash);

    let missing = project.read_symbol(&ReadSymbolRequest {
        path: "src/auth.rs".to_owned(),
        symbol_path: "Session::nope".to_owned(),
        session_id: None,
    });
    assert_eq!(code(missing), ErrorCode::NotFound);
}

#[test]
fn read_file_full_truncated_and_with_deps() {
    let (_dir, project) = setup();
    let full = ok(project.read_file(&read("src/auth.rs")));
    assert_eq!(full.content, AUTH_RS);
    assert!(!full.truncated);
    assert_eq!(
        full.file_hash,
        yusra_index::hash_bytes(AUTH_RS.as_bytes()).to_hex()
    );

    let cut = ok(project.read_file(&ReadFileRequest {
        max_tokens: Some(12),
        ..read("src/auth.rs")
    }));
    assert!(cut.truncated);
    assert!(cut.tokens <= 12);
    assert!(AUTH_RS.starts_with(&cut.content));
    assert!(cut.content.ends_with('\n'), "dipotong di akhir baris");
    assert_eq!(cut.file_hash, full.file_hash);

    let deps = ok(project.read_file(&ReadFileRequest {
        with_deps: true,
        ..read("src/auth.rs")
    }));
    let ctx = deps.deps_context.unwrap_or_default();
    assert!(ctx.contains("src/token.rs:"), "{ctx}");
    assert!(ctx.contains("pub fn issue_token"), "{ctx}");
    assert!(deps.tokens > full.tokens);
}

#[test]
fn project_map_and_estimate() {
    let (_dir, project) = setup();
    let map = ok(project.project_map(&GetProjectMapRequest {
        focus_paths: vec![],
        mention_idents: vec![],
        max_tokens: None,
    }));
    assert_eq!(map.files_total, 2);
    assert!(map.map.contains("issue_token"), "{}", map.map);

    let est = ok(project.estimate_tokens(&EstimateTokensRequest {
        text: Some("hello world".to_owned()),
        paths: vec!["src/token.rs".to_owned()],
        model_family: ModelFamilyParam::OpenAi,
    }));
    assert!(est.tokens > 2);
    assert!(!est.approximate);
}

#[test]
fn security_rejections() {
    let (_dir, project) = setup();
    for (path, expected) in [
        ("../etc/passwd", ErrorCode::InvalidPath),
        ("/etc/passwd", ErrorCode::InvalidPath),
        ("a/../../b", ErrorCode::InvalidPath),
        ("src\\auth.rs", ErrorCode::InvalidPath),
        (".env", ErrorCode::PathDenied),
        ("config/secrets.py", ErrorCode::PathDenied),
        (".git/config", ErrorCode::PathDenied),
        (".yusra/state.db", ErrorCode::PathDenied),
        ("src/none.rs", ErrorCode::NotFound),
        ("src", ErrorCode::NotFound),
    ] {
        assert_eq!(code(project.read_file(&read(path))), expected, "{path}");
    }
    let est = project.estimate_tokens(&EstimateTokensRequest {
        text: None,
        paths: vec![".env".to_owned()],
        model_family: ModelFamilyParam::Generic,
    });
    assert_eq!(code(est), ErrorCode::PathDenied);

    let session = project.read_file(&ReadFileRequest {
        session_id: Some("01J0000000000000000000000".to_owned()),
        ..read("src/auth.rs")
    });
    assert_eq!(code(session), ErrorCode::NotFound);
}

#[cfg(unix)]
#[test]
fn symlinks_leaving_root_or_to_denied_are_rejected() {
    use std::os::unix::fs::symlink;
    let (dir, project) = setup();
    let outside = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    write(outside.path(), "passwd", "root:x:0:0\n");
    let root = dir.path();
    for (link, target) in [
        ("src/out", outside.path().join("passwd")),
        ("src/env_link", root.join(".env")),
    ] {
        if let Err(e) = symlink(target, root.join(link)) {
            panic!("{e}");
        }
        assert_eq!(
            code(project.read_file(&read(link))),
            ErrorCode::PathDenied,
            "{link}"
        );
    }
    if let Err(e) = symlink(root.join("src/token.rs"), root.join("src/inside")) {
        panic!("{e}");
    }
    assert_eq!(ok(project.read_file(&read("src/inside"))).content, TOKEN_RS);
}

#[test]
fn apply_changes_updates_index() {
    let (dir, project) = setup();
    let root = dir.path();
    write(root, "src/extra/new.rs", "pub fn brand_new() {}\n");
    write(root, "src/extra/more.rs", "pub fn more_things() {}\n");
    let rel = |p: &str| match RelPath::parse(p) {
        Ok(p) => p,
        Err(e) => panic!("{e}"),
    };
    let stats = ok(project.apply_changes(&[
        Change::Upsert(rel("src/extra/new.rs")),
        Change::Upsert(rel("src/extra/more.rs")),
        Change::Upsert(rel(".env")),
    ]));
    assert_eq!(stats.updated, 2);
    assert_eq!(stats.files, 4);
    let found = |q: &str| {
        ok(project.search_symbols(&SearchSymbolsRequest {
            query: q.to_owned(),
            kind: None,
            limit: None,
        }))
        .symbols
        .len()
    };
    assert_eq!(found("brand_new"), 1);

    if let Err(e) = fs::remove_dir_all(root.join("src/extra")) {
        panic!("{e}");
    }
    let stats = ok(project.apply_changes(&[Change::Remove(rel("src/extra"))]));
    assert_eq!(stats.removed, 2);
    assert_eq!(found("brand_new"), 0);
    assert_eq!(ok(project.store().file_hashes()).len(), 2);

    // File yang hilang sebelum sempat di-index dianggap terhapus.
    let stats = ok(project.apply_changes(&[Change::Upsert(rel("src/ghost.rs"))]));
    assert_eq!(stats.updated, 0);
}
