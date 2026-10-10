#![cfg(feature = "lang-rust")]

use std::fmt::Write;

use yusra_core::{RelPath, Symbol, Tag};
use yusra_index::{LanguageAdapter, RustAdapter, adapter_for, builtin_adapters, hash_bytes, parse};

const SAMPLE: &str = include_str!("fixtures/rust/sample.rs");

fn rel(raw: &str) -> RelPath {
    match RelPath::parse(raw) {
        Ok(p) => p,
        Err(e) => panic!("{e}"),
    }
}

fn adapter() -> RustAdapter {
    match RustAdapter::new() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    }
}

fn tree(adapter: &RustAdapter, src: &str) -> tree_sitter::Tree {
    match parse(adapter, src.as_bytes()) {
        Ok(t) => t,
        Err(e) => panic!("{e}"),
    }
}

fn line_of(src: &str, byte: usize) -> usize {
    src[..byte].matches('\n').count() + 1
}

/// Satu baris per simbol: jenis, path, rentang baris, signature.
fn render_symbols(src: &str, symbols: &[Symbol]) -> String {
    let mut out = String::new();
    for s in symbols {
        let start = line_of(src, s.byte_range.start);
        let end = line_of(src, s.byte_range.end);
        let sig = s.signature.as_deref().unwrap_or("-");
        let _ = writeln!(out, "{} {} L{start}-{end} | {sig}", s.kind, s.symbol_path);
    }
    out
}

#[test]
fn symbols_from_fixture() {
    let a = adapter();
    let t = tree(&a, SAMPLE);
    assert!(!t.root_node().has_error(), "fixture harus valid");
    let path = rel("src/sample.rs");
    let symbols = a.symbols(&path, &t, SAMPLE.as_bytes());

    for s in &symbols {
        assert_eq!(s.path, path);
        assert_eq!(
            s.body_hash,
            hash_bytes(&SAMPLE.as_bytes()[s.byte_range.clone()])
        );
    }
    insta::assert_snapshot!(render_symbols(SAMPLE, &symbols));
}

#[test]
fn doc_comment_attached_only_without_blank_line() {
    let a = adapter();
    let t = tree(&a, SAMPLE);
    let symbols = a.symbols(&rel("src/sample.rs"), &t, SAMPLE.as_bytes());
    let source_of = |name: &str| -> &str {
        match symbols.iter().find(|s| s.symbol_path.as_str() == name) {
            Some(s) => &SAMPLE[s.byte_range.clone()],
            None => panic!("simbol {name} tidak ada"),
        }
    };
    assert!(source_of("Session").starts_with("/// Session login.\n#[derive"));
    assert!(source_of("Session::new").starts_with("/// Buat session baru."));
    assert!(source_of("Session::refresh").starts_with("pub fn refresh"));
    assert!(source_of("make_token").starts_with("fn make_token"));
    assert!(source_of("auth::platform").starts_with("#[cfg(unix)]"));
}

#[test]
fn imports_from_fixture() {
    let a = adapter();
    let t = tree(&a, SAMPLE);
    insta::assert_debug_snapshot!(a.imports(&t, SAMPLE.as_bytes()));
}

#[test]
fn tags_from_fixture() {
    let a = adapter();
    let t = tree(&a, SAMPLE);
    let tags = a.tags(&rel("src/sample.rs"), &t, SAMPLE.as_bytes());
    let mut out = String::new();
    for tag in &tags {
        let _ = match tag {
            Tag::Def { name, line, .. } => writeln!(out, "def {name} L{line}"),
            Tag::Ref { name, .. } => writeln!(out, "ref {name}"),
        };
    }
    insta::assert_snapshot!(out);
}

#[test]
fn registry_picks_by_extension() {
    let adapters = match builtin_adapters() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    };
    let id = |p: &str| adapter_for(&adapters, &rel(p)).map(|a| a.id());
    assert_eq!(id("src/main.rs"), Some("rust"));
    assert_eq!(id("README.md"), None);
    assert_eq!(id("Makefile"), None);
    assert_eq!(id("src/main.RS"), None);
}
