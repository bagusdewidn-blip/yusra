#![allow(dead_code, reason = "tiap file uji memakai sebagian helper")]

use std::fmt::Write;

use yusra_core::{RelPath, Symbol, Tag};
use yusra_index::{LanguageAdapter, hash_bytes, parse};

pub fn rel(raw: &str) -> RelPath {
    match RelPath::parse(raw) {
        Ok(p) => p,
        Err(e) => panic!("{e}"),
    }
}

pub fn tree(adapter: &dyn LanguageAdapter, src: &str) -> tree_sitter::Tree {
    let tree = match parse(adapter, src.as_bytes()) {
        Ok(t) => t,
        Err(e) => panic!("{e}"),
    };
    assert!(!tree.root_node().has_error(), "fixture harus valid");
    tree
}

fn line_of(src: &str, byte: usize) -> usize {
    src[..byte].matches('\n').count() + 1
}

/// Satu baris per simbol: jenis, path, rentang baris, signature. Sekaligus
/// memastikan `path` dan `body_hash` setiap simbol benar.
pub fn render_symbols(path: &RelPath, src: &str, symbols: &[Symbol]) -> String {
    let mut out = String::new();
    for s in symbols {
        assert_eq!(&s.path, path);
        assert_eq!(
            s.body_hash,
            hash_bytes(&src.as_bytes()[s.byte_range.clone()])
        );
        let start = line_of(src, s.byte_range.start);
        let end = line_of(src, s.byte_range.end);
        let sig = s.signature.as_deref().unwrap_or("-");
        let _ = writeln!(out, "{} {} L{start}-{end} | {sig}", s.kind, s.symbol_path);
    }
    out
}

pub fn render_tags(tags: &[Tag]) -> String {
    let mut out = String::new();
    for tag in tags {
        let _ = match tag {
            Tag::Def { name, line, .. } => writeln!(out, "def {name} L{line}"),
            Tag::Ref { name, .. } => writeln!(out, "ref {name}"),
        };
    }
    out
}

/// Simbol, impor, dan tag satu fixture dalam satu teks untuk snapshot.
pub fn render_all(adapter: &dyn LanguageAdapter, path: &str, src: &str) -> String {
    let path = rel(path);
    let t = tree(adapter, src);
    let symbols = adapter.symbols(&path, &t, src.as_bytes());
    let imports = adapter.imports(&t, src.as_bytes());
    let tags = adapter.tags(&path, &t, src.as_bytes());
    format!(
        "# simbol\n{}\n# impor\n{}\n\n# tag\n{}",
        render_symbols(&path, src, &symbols),
        imports.join("\n"),
        render_tags(&tags)
    )
}

/// Isi sumber simbol dengan path tertentu.
pub fn source_of<'a>(src: &'a str, symbols: &[Symbol], symbol_path: &str) -> &'a str {
    match symbols
        .iter()
        .find(|s| s.symbol_path.as_str() == symbol_path)
    {
        Some(s) => &src[s.byte_range.clone()],
        None => panic!("simbol {symbol_path} tidak ada"),
    }
}
