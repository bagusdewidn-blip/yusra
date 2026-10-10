use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use tree_sitter::{Language, Node, Query, QueryCursor, StreamingIterator, Tree};
use yusra_core::{RelPath, Symbol, SymbolKind, SymbolPath, Tag, YusraError};

use crate::{LanguageAdapter, hash_bytes};

const IMPORTS_QUERY: &str = r"
(use_declaration argument: (_) @import)
(mod_item name: (identifier) @module !body)
";

/// Path simbol Rust:
/// - item di modul: `auth::login`
/// - method inherent: `Session::refresh`
/// - method impl trait: `<Session as Display>::fmt`, agar `Display::fmt` dan
///   `Debug::fmt` pada tipe yang sama tidak bertabrakan
/// - blok impl itu sendiri: `impl Session`, `impl Display for Session`
///
/// Generic dibuang dari nama tipe dan trait (`Foo<T>` menjadi `Foo`).
pub struct RustAdapter {
    language: Language,
    tags_query: Query,
    imports_query: Query,
}

impl RustAdapter {
    pub fn new() -> Result<Self, YusraError> {
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let tags_query = Query::new(&language, tree_sitter_rust::TAGS_QUERY)
            .map_err(|e| YusraError::internal_with("query tags Rust tidak valid", e))?;
        let imports_query = Query::new(&language, IMPORTS_QUERY)
            .map_err(|e| YusraError::internal_with("query impor Rust tidak valid", e))?;
        Ok(Self {
            language,
            tags_query,
            imports_query,
        })
    }
}

impl LanguageAdapter for RustAdapter {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn language(&self) -> Language {
        self.language.clone()
    }

    fn symbols(&self, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Symbol> {
        let mut walker = SymbolWalker {
            path,
            src,
            scope: Vec::new(),
            out: Vec::new(),
        };
        walker.items(tree.root_node(), Container::Module);
        let mut symbols = walker.out;
        dedupe_symbol_paths(&mut symbols);
        symbols
    }

    fn imports(&self, tree: &Tree, src: &[u8]) -> Vec<String> {
        let names = self.imports_query.capture_names();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.imports_query, tree.root_node(), src);
        let mut out = Vec::new();
        while let Some(m) = matches.next() {
            for c in m.captures {
                let text = collapse_whitespace(&text(c.node, src));
                match names.get(c.index as usize).copied() {
                    Some("import") => out.push(text),
                    // `mod x;` memuat file anak, jadi dicatat sebagai dependensi.
                    Some("module") => out.push(format!("self::{text}")),
                    _ => {}
                }
            }
        }
        out
    }

    fn tags(&self, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Tag> {
        let names = self.tags_query.capture_names();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.tags_query, tree.root_node(), src);
        let mut out = Vec::new();
        // Method cocok dengan pola `definition.method` dan `definition.function`.
        let mut seen_defs = HashSet::new();
        while let Some(m) = matches.next() {
            let mut name_node = None;
            let mut role = None;
            for c in m.captures {
                match names.get(c.index as usize).copied() {
                    Some("name") => name_node = Some(c.node),
                    other => role = other,
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
            } else if role.starts_with("reference.") {
                // Rujukan ganda tidak dibuang: jumlahnya dipakai sebagai bobot repo map.
                out.push(Tag::Ref {
                    name,
                    file: path.clone(),
                });
            }
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Module,
    Impl,
    Trait,
}

struct SymbolWalker<'a> {
    path: &'a RelPath,
    src: &'a [u8],
    scope: Vec<String>,
    out: Vec<Symbol>,
}

impl SymbolWalker<'_> {
    fn items(&mut self, parent: Node, container: Container) {
        let mut cursor = parent.walk();
        let children: Vec<Node> = parent.named_children(&mut cursor).collect();
        for node in children {
            match node.kind() {
                "function_item" | "function_signature_item" => {
                    let kind = match container {
                        Container::Module => SymbolKind::Function,
                        Container::Impl | Container::Trait => SymbolKind::Method,
                    };
                    self.push_named(node, kind);
                }
                "struct_item" | "union_item" => {
                    self.push_named(node, SymbolKind::Struct);
                }
                "enum_item" => {
                    self.push_named(node, SymbolKind::Enum);
                }
                "const_item" | "static_item" => {
                    self.push_named(node, SymbolKind::Const);
                }
                "mod_item" => {
                    if let Some(name) = self.push_named(node, SymbolKind::Module) {
                        self.nested(node, name, Container::Module);
                    }
                }
                "trait_item" => {
                    if let Some(name) = self.push_named(node, SymbolKind::Trait) {
                        self.nested(node, name, Container::Trait);
                    }
                }
                "impl_item" => self.impl_item(node),
                _ => {}
            }
        }
    }

    fn impl_item(&mut self, node: Node) {
        let Some(ty) = node.child_by_field_name("type") else {
            return;
        };
        let ty = type_name(ty, self.src);
        let (impl_name, member_scope) = match node.child_by_field_name("trait") {
            Some(tr) => {
                let tr = type_name(tr, self.src);
                (format!("impl {tr} for {ty}"), format!("<{ty} as {tr}>"))
            }
            None => (format!("impl {ty}"), ty),
        };
        self.push(node, SymbolKind::Impl, &impl_name);
        self.nested(node, member_scope, Container::Impl);
    }

    fn nested(&mut self, node: Node, scope: String, container: Container) {
        if let Some(body) = node.child_by_field_name("body") {
            self.scope.push(scope);
            self.items(body, container);
            self.scope.pop();
        }
    }

    /// Mengembalikan nama simbol agar pemanggil bisa masuk ke isinya.
    fn push_named(&mut self, node: Node, kind: SymbolKind) -> Option<String> {
        let name = text(node.child_by_field_name("name")?, self.src).into_owned();
        self.push(node, kind, &name);
        Some(name)
    }

    fn push(&mut self, node: Node, kind: SymbolKind, name: &str) {
        let range = attached_start(node, self.src)..node.end_byte();
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
            signature: signature(node, self.src),
        });
    }
}

/// Awal simbol termasuk doc comment luar (`///`, `/** */`) dan atribut yang
/// menempel tanpa baris kosong. Komentar biasa dan `//!` tidak ikut.
fn attached_start(node: Node, src: &[u8]) -> usize {
    let mut start = node.start_byte();
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        let body = src.get(p.byte_range()).unwrap_or_default();
        let gap = src.get(p.end_byte()..start).unwrap_or_default();
        // Node `line_comment` bisa mencakup newline penutupnya sendiri.
        let newlines =
            gap.iter().filter(|&&b| b == b'\n').count() + usize::from(body.ends_with(b"\n"));
        let attached = is_outer_doc_or_attribute(p.kind(), body)
            && gap.iter().all(u8::is_ascii_whitespace)
            && newlines <= 1;
        if !attached {
            break;
        }
        start = p.start_byte();
        prev = p.prev_named_sibling();
    }
    start
}

fn is_outer_doc_or_attribute(kind: &str, body: &[u8]) -> bool {
    match kind {
        "attribute_item" => true,
        "line_comment" => body.starts_with(b"///") && !body.starts_with(b"////"),
        "block_comment" => {
            body.starts_with(b"/**") && !body.starts_with(b"/***") && body != b"/**/"
        }
        _ => false,
    }
}

/// Teks dari awal item sampai sebelum isi (`body`) atau nilai (`value`),
/// spasi dirapatkan. Contoh: `pub fn refresh(&mut self) -> bool`.
fn signature(node: Node, src: &[u8]) -> Option<String> {
    let end = node
        .child_by_field_name("body")
        .or_else(|| node.child_by_field_name("value"))
        .map_or(node.end_byte(), |n| n.start_byte());
    let raw = String::from_utf8_lossy(src.get(node.start_byte()..end)?);
    let sig = collapse_whitespace(&raw);
    // `,` tersisa dari klausa `where` multi-baris.
    let sig = sig.trim_end_matches([';', '=', ',', ' ']);
    (!sig.is_empty()).then(|| sig.to_owned())
}

/// Nama tipe tanpa generic, referensi, atau path modul.
fn type_name(node: Node, src: &[u8]) -> String {
    let inner = match node.kind() {
        "generic_type" | "reference_type" => node.child_by_field_name("type"),
        "scoped_type_identifier" => node.child_by_field_name("name"),
        _ => None,
    };
    match inner {
        Some(n) => type_name(n, src),
        None => collapse_whitespace(&text(node, src)),
    }
}

fn dedupe_symbol_paths(symbols: &mut [Symbol]) {
    let mut seen: HashMap<String, u32> = HashMap::new();
    for s in symbols {
        let n = seen.entry(s.symbol_path.0.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            s.symbol_path.0 = format!("{}#{n}", s.symbol_path.0);
        }
    }
}

fn text<'a>(node: Node, src: &'a [u8]) -> Cow<'a, str> {
    String::from_utf8_lossy(src.get(node.byte_range()).unwrap_or_default())
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
