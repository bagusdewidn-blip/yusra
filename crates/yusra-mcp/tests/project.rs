use std::{fs, path::Path};

use yusra_core::{ErrorCode, RelPath, YusraError};
use yusra_index::Change;
use yusra_mcp::{
    EstimateTokensRequest, GetProjectMapRequest, ModelFamilyParam, Project, ReadFileRequest,
    ReadSymbolRequest, STATE_DIR, SearchSymbolsRequest, Settings, SymbolKindParam,
};
use yusra_store::{FileRecord, FileTagRows, TagDef};

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

fn setup_unindexed() -> (tempfile::TempDir, Project) {
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
    let project = ok(Project::open(root, test_settings()));
    (dir, project)
}

fn test_settings() -> Settings {
    Settings {
        deny_read: [".env", "config/secrets.py"].map(str::to_owned).to_vec(),
        ..Settings::default()
    }
}

fn map_text(project: &Project) -> String {
    ok(project.project_map(&GetProjectMapRequest {
        focus_paths: vec!["src/auth.rs".to_owned()],
        mention_idents: vec![],
        max_tokens: None,
    }))
    .map
}

fn setup() -> (tempfile::TempDir, Project) {
    let (dir, project) = setup_unindexed();
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
fn read_file_with_deps_keeps_room_for_deps_when_truncated() {
    let (dir, project) = setup();
    let filler: String = (0..200)
        .map(|i| format!("// baris pengisi nomor {i}\n"))
        .collect();
    let big = format!("{AUTH_RS}{filler}");
    write(dir.path(), "src/auth.rs", &big);
    ok(project.apply_changes(&[Change::Upsert(ok(RelPath::parse("src/auth.rs")))]));

    let budget = 200;
    let resp = ok(project.read_file(&ReadFileRequest {
        with_deps: true,
        max_tokens: Some(budget),
        ..read("src/auth.rs")
    }));
    assert!(resp.truncated);
    assert!(big.starts_with(&resp.content));
    assert!(resp.tokens <= budget, "{} > {budget}", resp.tokens);
    let ctx = resp.deps_context.unwrap_or_default();
    assert!(ctx.contains("pub fn issue_token"), "{ctx}");
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

#[test]
fn wait_ready_blocks_until_initial_index() {
    let (_dir, project) = setup_unindexed();
    let project = std::sync::Arc::new(project);
    let waiter = {
        let project = std::sync::Arc::clone(&project);
        std::thread::spawn(move || {
            project.wait_ready()?;
            project.search_symbols(&SearchSymbolsRequest {
                query: "issue_token".to_owned(),
                kind: None,
                limit: None,
            })
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !waiter.is_finished(),
        "wait_ready harus memblok sebelum index"
    );
    ok(project.full_index());
    let found = match waiter.join() {
        Ok(r) => ok(r),
        Err(_) => panic!("thread penunggu panic"),
    };
    assert_eq!(found.symbols.len(), 1, "{found:?}");
}

#[test]
fn failed_initial_index_reaches_waiters() {
    let (_dir, project) = setup_unindexed();
    project.mark_index_failed("disk penuh");
    assert_eq!(code(project.wait_ready()), ErrorCode::Internal);
    // Index yang berhasil kemudian (misalnya rescan) memulihkan keadaan.
    ok(project.full_index());
    ok(project.wait_ready());
    project.mark_index_failed("terlambat");
    ok(project.wait_ready());
}

#[test]
fn full_index_spans_several_parallel_batches() {
    let (dir, project) = setup_unindexed();
    for i in 0..600 {
        write(
            dir.path(),
            &format!("src/gen/m{i:03}.rs"),
            &format!("pub fn generated_fn_{i:03}() {{}}\n"),
        );
    }
    let stats = ok(project.full_index());
    // 600 file baru + src/auth.rs + src/token.rs; .env dan secrets dilewati.
    assert_eq!(stats.files, 602, "{stats:?}");
    assert_eq!(stats.updated, 602, "{stats:?}");
    assert_eq!(stats.skipped, 2, "{stats:?}");
    for i in [0, 255, 256, 599] {
        let found = ok(project.search_symbols(&SearchSymbolsRequest {
            query: format!("generated_fn_{i:03}"),
            kind: None,
            limit: None,
        }));
        assert_eq!(found.symbols.len(), 1, "{i}: {found:?}");
        assert_eq!(found.symbols[0].path, format!("src/gen/m{i:03}.rs"));
    }
    let again = ok(project.full_index());
    assert_eq!((again.files, again.updated), (602, 0), "{again:?}");
}

#[test]
fn reopen_reuses_stored_tags_without_reparse() {
    let (dir, project) = setup();
    let before = map_text(&project);
    assert!(before.contains("pub fn issue_token"), "{before}");
    drop(project);

    let again = ok(Project::open(dir.path(), test_settings()));
    let stats = ok(again.full_index());
    assert_eq!((stats.files, stats.updated), (2, 0), "{stats:?}");
    assert_eq!(map_text(&again), before, "tag dari database harus identik");

    // File yang berubah saat daemon mati tetap diparse ulang.
    write(
        dir.path(),
        "src/token.rs",
        "pub fn mint_token() {}
",
    );
    drop(again);
    let third = ok(Project::open(dir.path(), test_settings()));
    let stats = ok(third.full_index());
    assert_eq!((stats.files, stats.updated), (2, 1), "{stats:?}");
    let map = map_text(&third);
    assert!(!map.contains("pub fn issue_token"), "{map}");
    let found = ok(third.search_symbols(&SearchSymbolsRequest {
        query: "mint_token".to_owned(),
        kind: None,
        limit: None,
    }));
    assert_eq!(found.symbols.len(), 1, "{found:?}");
}

#[test]
fn changed_indexer_settings_rebuild_index() {
    let (dir, project) = setup();
    drop(project);
    // Batas ukuran parse berbeda mengubah sidik indexer.
    let settings = Settings {
        max_file_bytes: 4096,
        ..test_settings()
    };
    let again = ok(Project::open(dir.path(), settings.clone()));
    let stats = ok(again.full_index());
    assert_eq!((stats.files, stats.updated), (2, 2), "{stats:?}");
    drop(again);
    let same = ok(Project::open(dir.path(), settings));
    let stats = ok(same.full_index());
    assert_eq!(stats.updated, 0, "{stats:?}");
}

#[test]
fn unchanged_file_takes_tags_from_database() {
    let (dir, project) = setup();
    // Ganti tag tersimpan token.rs tanpa mengubah hash-nya. Jika full_index
    // berikutnya mem-parse ulang, teks ini tidak akan muncul.
    let path = ok(RelPath::parse("src/token.rs"));
    let tags = FileTagRows {
        defs: vec![TagDef {
            name: "issue_token".to_owned(),
            line: 1,
            text: "pub fn issue_token /* dari database */".to_owned(),
        }],
        refs: vec![],
    };
    ok(project.store().replace_file(&FileRecord {
        path: &path,
        hash: yusra_index::hash_bytes(TOKEN_RS.as_bytes()),
        language: Some("rust"),
        indexed_at: 0,
        symbols: &[],
        deps: &[],
        tags: &tags,
    }));
    drop(project);

    let again = ok(Project::open(dir.path(), test_settings()));
    ok(again.full_index());
    let map = map_text(&again);
    assert!(map.contains("/* dari database */"), "{map}");
}

#[test]
fn read_file_truncation_fills_budget_closely() {
    let (dir, project) = setup();
    let body: String = (0..3000)
        .map(|i| format!("    let value_{i} = compute_{i}(input, {i});\n"))
        .collect();
    write(dir.path(), "src/long.rs", &body);
    let one_line = "token ".repeat(20_000);
    write(dir.path(), "src/one_line.txt", &one_line);

    for (path, src) in [
        ("src/long.rs", body.as_str()),
        ("src/one_line.txt", &one_line),
    ] {
        let budget = 500;
        let resp = ok(project.read_file(&ReadFileRequest {
            max_tokens: Some(budget),
            ..read(path)
        }));
        assert!(resp.truncated, "{path}");
        assert!(src.starts_with(&resp.content), "{path}");
        assert!(resp.tokens <= budget, "{path}: {} > {budget}", resp.tokens);
        // Pemotongan per baris boleh sedikit kurang dari anggaran, tapi
        // tidak boleh membuang sebagian besar anggaran.
        assert!(
            resp.tokens >= budget * 9 / 10,
            "{path}: hanya {}",
            resp.tokens
        );
    }
}
