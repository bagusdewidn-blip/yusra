use yusra_core::{Hash32, RelPath, SymbolPath, YusraError};
use yusra_index::{builtin_adapters, hash_bytes};
use yusra_session::{Editor, FileView, check_path, lock_covers};

const LIB: &str = r#"pub struct Session {
    id: u64,
}

impl Session {
    pub fn new(id: u64) -> Self {
        Self { id }
    }

    pub fn refresh(&mut self) {
        self.id += 1;
    }
}

/// Masuk.
pub fn login() -> Session {
    Session::new(1)
}
"#;

fn ok<T, E: std::fmt::Display>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

fn rel(raw: &str) -> RelPath {
    ok(RelPath::parse(raw))
}

fn sp(raw: &str) -> SymbolPath {
    SymbolPath(raw.to_owned())
}

fn editor() -> Editor {
    Editor::new(ok(builtin_adapters()).into(), 1024 * 1024)
}

fn view(e: &Editor, path: &str, content: Option<&str>) -> FileView {
    ok(e.view(&rel(path), content.map(|c| c.as_bytes().to_vec())))
}

fn hash_of(v: &FileView, symbol: &str) -> Hash32 {
    ok(v.symbol(&sp(symbol))).body_hash
}

fn text(v: &FileView) -> String {
    String::from_utf8_lossy(v.bytes().unwrap_or_default()).into_owned()
}

#[test]
fn edit_symbol_splices_and_reports_new_symbol() {
    let e = editor();
    let v = view(&e, "src/lib.rs", Some(LIB));
    let base = hash_of(&v, "login");
    let new_login = "/// Masuk sebagai tamu.\npub fn login() -> Session {\n    Session::new(0)\n}";

    let edited = ok(e.edit_symbol(&v, &sp("login"), base, new_login));
    let out = text(&edited.view);
    assert!(out.contains("Session::new(0)"));
    assert!(
        !out.contains("/// Masuk.\n"),
        "doc comment lama ikut diganti"
    );
    assert!(out.starts_with("pub struct Session"), "bagian lain tetap");
    assert_eq!(edited.base_hash, Some(base));
    assert_eq!(edited.new_hash, hash_bytes(new_login.as_bytes()));
    let symbol = match &edited.symbol {
        Some(s) => s,
        None => panic!("simbol hasil edit harus ditemukan"),
    };
    assert_eq!(symbol.body_hash, hash_bytes(new_login.as_bytes()));
    assert_eq!(edited.view.hash(), Some(hash_bytes(out.as_bytes())));
}

#[test]
fn edit_symbol_rejections() {
    let e = editor();
    let v = view(&e, "src/lib.rs", Some(LIB));
    let base = hash_of(&v, "login");

    match e.edit_symbol(&v, &sp("login"), Hash32([0; 32]), "pub fn login() {}") {
        Err(YusraError::StaleBase { actual, .. }) => assert_eq!(actual, Some(base)),
        other => panic!("harus stale_base, hasil {other:?}"),
    }
    assert!(matches!(
        e.edit_symbol(&v, &sp("login"), base, "pub fn login( -> {"),
        Err(YusraError::ParseError { .. })
    ));
    assert!(matches!(
        e.edit_symbol(&v, &sp("tidak_ada"), base, "x"),
        Err(YusraError::NotFound { kind: "simbol", .. })
    ));
    assert!(matches!(
        e.edit_symbol(&v, &SymbolPath::whole_file(), base, "x"),
        Err(YusraError::PatchRejected { .. })
    ));

    let missing = view(&e, "src/gone.rs", None);
    assert!(matches!(
        e.edit_symbol(&missing, &sp("login"), base, "x"),
        Err(YusraError::NotFound { kind: "file", .. })
    ));
    let readme = view(&e, "README.md", Some("# judul\n"));
    assert!(matches!(
        e.edit_symbol(&readme, &sp("judul"), base, "x"),
        Err(YusraError::PatchRejected { .. })
    ));
    let tiny = Editor::new(ok(builtin_adapters()).into(), 8);
    let big = view(&tiny, "src/lib.rs", Some(LIB));
    assert!(matches!(
        tiny.edit_symbol(&big, &sp("login"), base, "x"),
        Err(YusraError::PatchRejected { .. })
    ));
}

#[test]
fn already_broken_files_can_still_be_edited() {
    let e = editor();
    let broken = format!("{LIB}\nfn rusak( {{\n");
    let v = view(&e, "src/lib.rs", Some(&broken));
    assert!(v.has_errors());
    let base = hash_of(&v, "login");
    ok(e.edit_symbol(
        &v,
        &sp("login"),
        base,
        "pub fn login() -> Session {\n    Session::new(2)\n}",
    ));
}

#[test]
fn replace_file_create_overwrite_delete() {
    let e = editor();
    let absent = view(&e, "src/auth.rs", None);
    let created = ok(e.replace_file(&absent, None, Some(b"pub fn a() {}\n")));
    assert_eq!(created.base_hash, None);
    assert_eq!(created.view.hash(), Some(hash_bytes(b"pub fn a() {}\n")));
    assert_eq!(created.view.symbols().len(), 1);

    let existing = created.view;
    let hash = existing.hash();
    assert!(matches!(
        e.replace_file(&existing, None, Some(b"x")),
        Err(YusraError::PatchRejected { .. })
    ));
    assert!(matches!(
        e.replace_file(&existing, Some(Hash32([0; 32])), Some(b"x")),
        Err(YusraError::StaleBase { .. })
    ));
    assert!(matches!(
        e.replace_file(&existing, hash, Some(b"pub fn a( {\n")),
        Err(YusraError::ParseError { .. })
    ));

    let deleted = ok(e.replace_file(&existing, hash, None));
    assert_eq!(deleted.view.bytes(), None);
    assert_eq!(deleted.base_hash, hash);
    assert_eq!(deleted.new_hash, hash_bytes(b""));
    assert!(matches!(
        e.replace_file(&deleted.view, None, None),
        Err(YusraError::NotFound { kind: "file", .. })
    ));
    let Some(old) = hash else {
        panic!("file harus punya hash");
    };
    assert!(matches!(
        e.replace_file(&deleted.view, Some(old), None),
        Err(YusraError::StaleBase { actual: None, .. })
    ));

    // File non-kode tidak diparse, jadi isi apa pun diterima.
    let readme = view(&e, "README.md", Some("# demo\n"));
    ok(e.replace_file(&readme, readme.hash(), Some(b"# baru ( {\n")));
}

#[test]
fn internal_paths_are_denied() {
    let e = editor();
    for path in [".git/config", ".yusra/state.db", "vendor/.git/HEAD"] {
        assert!(
            matches!(check_path(&rel(path)), Err(YusraError::PathDenied { .. })),
            "{path} harus ditolak"
        );
        let v = view(&e, path, None);
        assert!(matches!(
            e.replace_file(&v, None, Some(b"x")),
            Err(YusraError::PathDenied { .. })
        ));
    }
    assert!(check_path(&rel("docs/.yusra/notes.md")).is_ok());
    assert!(check_path(&rel(".github/workflows/ci.yml")).is_ok());
}

#[test]
fn parent_lock_covers_methods_in_parsed_view() {
    let e = editor();
    let v = view(&e, "src/lib.rs", Some(LIB));
    assert!(lock_covers(
        &sp("impl Session"),
        &sp("Session::refresh"),
        v.symbols()
    ));
    assert!(!lock_covers(
        &sp("login"),
        &sp("Session::refresh"),
        v.symbols()
    ));
}
