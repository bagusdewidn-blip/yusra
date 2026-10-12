use tree_sitter::{Language, Node, Query, Tree};
use yusra_core::{RelPath, Symbol, SymbolKind, Tag, YusraError};

use crate::{
    LanguageAdapter,
    common::{
        self, SymbolSink, attached_start, collapse_whitespace, named_children, signature_between,
        text,
    },
};

const IMPORTS_QUERY: &str = r"
(use_declaration argument: (_) @import)
(mod_item name: (identifier) @module !body)
";

/// Query tag bawaan tree-sitter-rust tidak menangkap panggilan ber-path
/// (`Session::new()`) dan rujukan tipe, jadi dipakai query sendiri.
const TAGS_QUERY: &str = include_str!("../queries/rust-tags.scm");

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
        let tags_query = Query::new(&language, TAGS_QUERY)
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
            src,
            sink: SymbolSink::new(path, src),
        };
        walker.items(tree.root_node(), Container::Module);
        walker.sink.finish()
    }

    fn imports(&self, tree: &Tree, src: &[u8]) -> Vec<String> {
        common::captures(&self.imports_query, tree, src)
            .into_iter()
            .map(|(name, text)| match name {
                // `mod x;` memuat file anak, jadi dicatat sebagai dependensi.
                "module" => format!("self::{text}"),
                _ => text,
            })
            .collect()
    }

    fn tags(&self, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Tag> {
        common::tags(&self.tags_query, path, tree, src)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Module,
    Impl,
    Trait,
}

struct SymbolWalker<'a> {
    src: &'a [u8],
    sink: SymbolSink<'a>,
}

impl SymbolWalker<'_> {
    fn items(&mut self, parent: Node, container: Container) {
        for node in named_children(parent) {
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
            self.sink.enter(scope);
            self.items(body, container);
            self.sink.leave();
        }
    }

    /// Mengembalikan nama simbol agar pemanggil bisa masuk ke isinya.
    fn push_named(&mut self, node: Node, kind: SymbolKind) -> Option<String> {
        let name = text(node.child_by_field_name("name")?, self.src).into_owned();
        self.push(node, kind, &name);
        Some(name)
    }

    fn push(&mut self, node: Node, kind: SymbolKind, name: &str) {
        let range = attached_start(node, self.src, is_outer_doc_or_attribute)..node.end_byte();
        // Signature berhenti sebelum isi (`body`) atau nilai (`value`).
        let sig_end = node
            .child_by_field_name("body")
            .or_else(|| node.child_by_field_name("value"))
            .map_or(node.end_byte(), |n| n.start_byte());
        let signature = signature_between(self.src, node.start_byte(), sig_end);
        self.sink.push(range, kind, name, signature);
    }
}

/// Doc comment luar (`///`, `/** */`) dan atribut. Komentar biasa dan `//!` tidak.
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
