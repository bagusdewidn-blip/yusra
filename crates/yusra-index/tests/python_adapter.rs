#![cfg(feature = "lang-python")]

mod common;

use common::{rel, render_all, source_of, tree};
use yusra_index::{LanguageAdapter, PythonAdapter, adapter_for, builtin_adapters};

const SAMPLE: &str = include_str!("fixtures/python/sample.py");

fn unwrap<T>(r: Result<T, yusra_core::YusraError>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn python_fixture() {
    let a = unwrap(PythonAdapter::new());
    insta::assert_snapshot!(render_all(&a, "app/sample.py", SAMPLE));
}

#[test]
fn decorators_and_docstrings_inside_range() {
    let a = unwrap(PythonAdapter::new());
    let t = tree(&a, SAMPLE);
    let symbols = a.symbols(&rel("app/sample.py"), &t, SAMPLE.as_bytes());
    let source = |name| source_of(SAMPLE, &symbols, name);
    assert!(source("Session").starts_with("@dataclass(frozen=True)\nclass Session"));
    assert!(source("Session").contains("\"\"\"Session login.\"\"\""));
    assert!(source("Session::active").starts_with("@property"));
    assert!(source("make_token").starts_with("def make_token"));
}

#[test]
fn registry_picks_python() {
    let adapters = unwrap(builtin_adapters());
    let id = |p: &str| adapter_for(&adapters, &rel(p)).map(|a| a.id());
    assert_eq!(id("app/main.py"), Some("python"));
    assert_eq!(id("app/stubs.pyi"), Some("python"));
    assert_eq!(id("app/main.pyc"), None);
}
