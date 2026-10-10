#![cfg(feature = "lang-typescript")]

mod common;

use common::{rel, render_all, source_of, tree};
use yusra_index::{LanguageAdapter, TypeScriptAdapter, adapter_for, builtin_adapters};

const SAMPLE_TS: &str = include_str!("fixtures/typescript/sample.ts");
const SAMPLE_TSX: &str = include_str!("fixtures/typescript/sample.tsx");
const SAMPLE_JS: &str = include_str!("fixtures/javascript/sample.js");

fn unwrap<T>(r: Result<T, yusra_core::YusraError>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn typescript_fixture() {
    let a = unwrap(TypeScriptAdapter::typescript());
    insta::assert_snapshot!(render_all(&a, "src/sample.ts", SAMPLE_TS));
}

#[test]
fn tsx_fixture() {
    let a = unwrap(TypeScriptAdapter::tsx());
    insta::assert_snapshot!(render_all(&a, "src/sample.tsx", SAMPLE_TSX));
}

#[test]
fn javascript_fixture() {
    let a = unwrap(TypeScriptAdapter::javascript());
    insta::assert_snapshot!(render_all(&a, "src/sample.js", SAMPLE_JS));
}

#[test]
fn jsdoc_and_decorators_attached() {
    let a = unwrap(TypeScriptAdapter::typescript());
    let t = tree(&a, SAMPLE_TS);
    let symbols = a.symbols(&rel("src/sample.ts"), &t, SAMPLE_TS.as_bytes());
    let source = |name| source_of(SAMPLE_TS, &symbols, name);
    assert!(source("Client").starts_with("/** Klien HTTP. */\n@Injectable"));
    assert!(source("Client::send").starts_with("/** Kirim permintaan. */"));
    assert!(source("Client::close").starts_with("@Log()"));
    assert!(source("Session").starts_with("/**\n * Session login."));
    assert!(source("MAX_RETRY").starts_with("/** Batas percobaan. */\nexport const"));
    assert!(source("makeToken").starts_with("export function makeToken"));
}

#[test]
fn registry_picks_dialect_by_extension() {
    let adapters = unwrap(builtin_adapters());
    let id = |p: &str| adapter_for(&adapters, &rel(p)).map(|a| a.id());
    assert_eq!(id("src/app.ts"), Some("typescript"));
    assert_eq!(id("src/types.d.ts"), Some("typescript"));
    assert_eq!(id("src/app.mts"), Some("typescript"));
    assert_eq!(id("src/App.tsx"), Some("tsx"));
    assert_eq!(id("src/app.js"), Some("javascript"));
    assert_eq!(id("src/App.jsx"), Some("javascript"));
    assert_eq!(id("src/app.cjs"), Some("javascript"));
}
