#![cfg(feature = "lang-rust")]

mod common;

use common::{rel, render_symbols, render_tags, source_of, tree};
use yusra_core::Tag;
use yusra_index::{LanguageAdapter, RustAdapter, adapter_for, builtin_adapters};

const SAMPLE: &str = include_str!("fixtures/rust/sample.rs");

fn adapter() -> RustAdapter {
    match RustAdapter::new() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn symbols_from_fixture() {
    let a = adapter();
    let t = tree(&a, SAMPLE);
    let path = rel("src/sample.rs");
    let symbols = a.symbols(&path, &t, SAMPLE.as_bytes());
    insta::assert_snapshot!(render_symbols(&path, SAMPLE, &symbols));
}

#[test]
fn doc_comment_attached_only_without_blank_line() {
    let a = adapter();
    let t = tree(&a, SAMPLE);
    let symbols = a.symbols(&rel("src/sample.rs"), &t, SAMPLE.as_bytes());
    let source = |name| source_of(SAMPLE, &symbols, name);
    assert!(source("Session").starts_with("/// Session login.\n#[derive"));
    assert!(source("Session::new").starts_with("/// Buat session baru."));
    assert!(source("Session::refresh").starts_with("pub fn refresh"));
    assert!(source("make_token").starts_with("fn make_token"));
    assert!(source("auth::platform").starts_with("#[cfg(unix)]"));
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
    insta::assert_snapshot!(render_tags(&tags));
}

#[test]
fn definition_positions_are_not_refs() {
    let a = adapter();
    let src = "struct A;\nenum B {}\nunion U { x: u8 }\ntrait C { type Out; }\n\
               type D = E;\nfn f<T>() -> Self {}\nmod m {}\n";
    let t = tree(&a, src);
    let refs: Vec<_> = a
        .tags(&rel("src/x.rs"), &t, src.as_bytes())
        .into_iter()
        .filter_map(|tag| match tag {
            Tag::Ref { name, .. } => Some(name),
            Tag::Def { .. } => None,
        })
        .collect();
    assert_eq!(refs, ["E"]);
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
