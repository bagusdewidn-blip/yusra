//! Bagian ekstraksi yang sama untuk semua bahasa.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    ops::Range,
};

use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};
use yusra_core::{RelPath, Symbol, SymbolKind, SymbolPath, Tag};

use crate::hash_bytes;

/// Pengumpul simbol dengan scope bersarang. Segmen scope digabung dengan `::`.
pub(crate) struct SymbolSink<'a> {
    path: &'a RelPath,
    src: &'a [u8],
    scope: Vec<String>,
    out: Vec<Symbol>,
}

impl<'a> SymbolSink<'a> {
    pub(crate) fn new(path: &'a RelPath, src: &'a [u8]) -> Self {
        Self {
            path,
            src,
            scope: Vec::new(),
            out: Vec::new(),
        }
    }

    pub(crate) fn push(
        &mut self,
        range: Range<usize>,
        kind: SymbolKind,
        name: &str,
        signature: Option<String>,
    ) {
        let body = self.src.get(range.clone()).unwrap_or_default();
        let symbol_path = self
            .scope
            .iter()
            .map(String::as_str)
            .chain([name])
            .collect::<Vec<_>>()
            .join("::");
        self.out.push(Symbol {
            path: self.path.clone(),
            symbol_path: SymbolPath(symbol_path),
            kind,
            body_hash: hash_bytes(body),
            byte_range: range,
            signature,
        });
    }

    pub(crate) fn enter(&mut self, scope: String) {
        self.scope.push(scope);
    }

    pub(crate) fn leave(&mut self) {
        self.scope.pop();
    }

    /// Nama ganda dalam satu file diberi akhiran `#2`, `#3` sesuai urutan sumber.
    pub(crate) fn finish(self) -> Vec<Symbol> {
        let mut symbols = self.out;
        let mut seen: HashMap<String, u32> = HashMap::new();
        for s in &mut symbols {
            let n = seen.entry(s.symbol_path.0.clone()).or_insert(0);
            *n += 1;
            if *n > 1 {
                s.symbol_path.0 = format!("{}#{n}", s.symbol_path.0);
            }
        }
        symbols
    }
}

/// Awal simbol termasuk sibling sebelumnya yang `attachable` (doc comment,
/// atribut, decorator) selama tidak dipisah baris kosong.
#[cfg(any(
    feature = "lang-rust",
    feature = "lang-typescript",
    feature = "lang-go"
))]
pub(crate) fn attached_start(
    node: Node,
    src: &[u8],
    attachable: impl Fn(&str, &[u8]) -> bool,
) -> usize {
    let mut start = node.start_byte();
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        let body = src.get(p.byte_range()).unwrap_or_default();
        let gap = src.get(p.end_byte()..start).unwrap_or_default();
        // Sebagian grammar memasukkan newline penutup ke node komentar baris.
        let newlines =
            gap.iter().filter(|&&b| b == b'\n').count() + usize::from(body.ends_with(b"\n"));
        let attached =
            attachable(p.kind(), body) && gap.iter().all(u8::is_ascii_whitespace) && newlines <= 1;
        if !attached {
            break;
        }
        start = p.start_byte();
        prev = p.prev_named_sibling();
    }
    start
}

/// Teks `src[start..end]` dengan spasi dirapatkan dan sisa sintaks di ujung
/// (`;`, `=`, `=>`, `,` dari klausa `where`, `:` sebelum blok Python) dibuang.
pub(crate) fn signature_between(src: &[u8], start: usize, end: usize) -> Option<String> {
    let raw = String::from_utf8_lossy(src.get(start..end)?);
    let sig = collapse_whitespace(&raw);
    let sig = sig.strip_suffix("=>").unwrap_or(&sig);
    let sig = sig.trim_end_matches([';', '=', ',', ':', ' ']);
    (!sig.is_empty()).then(|| sig.to_owned())
}

/// Tag dari query bergaya `tags.scm`: capture `@name` ditambah capture peran
/// `@definition.*` atau `@reference.*`. Capture lain (misalnya `@doc`) diabaikan.
pub(crate) fn tags(query: &Query, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Tag> {
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), src);
    let mut out = Vec::new();
    // Satu definisi bisa cocok dengan beberapa pola, misalnya method dan function.
    let mut seen_defs = HashSet::new();
    while let Some(m) = matches.next() {
        let mut name_node = None;
        let mut role = None;
        for c in m.captures {
            match names.get(c.index as usize).copied() {
                Some("name") => name_node = Some(c.node),
                Some(r) if r.starts_with("definition.") || r.starts_with("reference.") => {
                    role = Some(r);
                }
                _ => {}
            }
        }
        let (Some(node), Some(role)) = (name_node, role) else {
            continue;
        };
        let name = text(node, src).into_owned();
        if role.starts_with("definition.") {
            let line = u32::try_from(node.start_position().row + 1).unwrap_or(u32::MAX);
            if seen_defs.insert((name.clone(), line)) {
                out.push(Tag::Def {
                    name,
                    file: path.clone(),
                    line,
                });
            }
        } else {
            // Rujukan ganda tidak dibuang: jumlahnya dipakai sebagai bobot repo map.
            out.push(Tag::Ref {
                name,
                file: path.clone(),
            });
        }
    }
    out
}

/// Pasangan (nama capture, teks) dalam urutan kecocokan. Capture yang namanya
/// diawali `_` adalah bantuan predikat dan tidak dikembalikan.
#[cfg(any(
    feature = "lang-rust",
    feature = "lang-typescript",
    feature = "lang-go"
))]
pub(crate) fn captures<'q>(query: &'q Query, tree: &Tree, src: &[u8]) -> Vec<(&'q str, String)> {
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), src);
    let mut out = Vec::new();
    while let Some(m) = matches.next() {
        for c in m.captures {
            if let Some(&name) = names.get(c.index as usize)
                && !name.starts_with('_')
            {
                out.push((name, collapse_whitespace(&text(c.node, src))));
            }
        }
    }
    out
}

pub(crate) fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

pub(crate) fn text<'a>(node: Node, src: &'a [u8]) -> Cow<'a, str> {
    String::from_utf8_lossy(src.get(node.byte_range()).unwrap_or_default())
}

pub(crate) fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
