#![cfg(feature = "lang-go")]

mod common;

use common::{rel, render_all, source_of, tree};
use yusra_index::{GoAdapter, LanguageAdapter, adapter_for, builtin_adapters};

const SAMPLE: &str = include_str!("fixtures/go/sample.go");

fn unwrap<T>(r: Result<T, yusra_core::YusraError>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn go_fixture() {
    let a = unwrap(GoAdapter::new());
    insta::assert_snapshot!(render_all(&a, "pkg/sample.go", SAMPLE));
}

#[test]
fn doc_comments_attached() {
    let a = unwrap(GoAdapter::new());
    let t = tree(&a, SAMPLE);
    let symbols = a.symbols(&rel("pkg/sample.go"), &t, SAMPLE.as_bytes());
    let source = |name| source_of(SAMPLE, &symbols, name);
    assert!(source("MaxRetry").starts_with("// MaxRetry adalah"));
    assert!(source("Session").starts_with("// Session adalah session login.\ntype Session"));
    assert!(source("Session::Refresh").starts_with("// Refresh memperbarui"));
    assert!(source("Store").starts_with("// Store menyimpan"));
    assert!(source("StateActive").starts_with("// StateActive menandai"));
    assert!(source("StateClosed").starts_with("StateClosed"));
    assert!(source("handler").starts_with("func handler"));
}

#[test]
fn registry_picks_go() {
    let adapters = unwrap(builtin_adapters());
    let id = |p: &str| adapter_for(&adapters, &rel(p)).map(|a| a.id());
    assert_eq!(id("cmd/main.go"), Some("go"));
    assert_eq!(id("go.mod"), None);
}
