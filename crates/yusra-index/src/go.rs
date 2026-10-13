use tree_sitter::{Language, Node, Query, Tree};
use yusra_core::{RelPath, Symbol, SymbolKind, Tag, YusraError};

use crate::{
    LanguageAdapter, ProjectLayout,
    common::{self, SymbolSink, attached_start, named_children, signature_between, text},
};

pub(crate) const IMPORTS_QUERY: &str = "(import_spec path: (_) @import)";

/// Path simbol Go: `MakeToken`, `Session`, method `Session::Refresh` (receiver
/// pointer dan generic dibuang, `*Session[T]` menjadi `Session`).
///
/// Komentar yang menempel tepat di atas deklarasi adalah doc comment Go dan
/// ikut dalam rentang. Tipe dicatat jika struct atau interface; tipe bernama
/// lain (`type Status int`) tidak, sama seperti alias di bahasa lain.
/// Konstanta dari `const`; `var` tidak dicatat.
pub struct GoAdapter {
    language: Language,
    tags_query: Query,
    imports_query: Query,
}

impl GoAdapter {
    pub fn new() -> Result<Self, YusraError> {
        let language: Language = tree_sitter_go::LANGUAGE.into();
        let tags_query = Query::new(&language, tree_sitter_go::TAGS_QUERY)
            .map_err(|e| YusraError::internal_with("query tags Go tidak valid", e))?;
        let imports_query = Query::new(&language, IMPORTS_QUERY)
            .map_err(|e| YusraError::internal_with("query impor Go tidak valid", e))?;
        Ok(Self {
            language,
            tags_query,
            imports_query,
        })
    }
}

impl LanguageAdapter for GoAdapter {
    fn id(&self) -> &'static str {
        "go"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["go"]
    }

    fn language(&self) -> Language {
        self.language.clone()
    }

    fn symbols(&self, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Symbol> {
        let mut walker = SymbolWalker {
            src,
            sink: SymbolSink::new(path, src),
        };
        walker.source_file(tree.root_node());
        walker.sink.finish()
    }

    /// Path impor tanpa tanda kutip, misalnya `net/http`.
    fn imports(&self, tree: &Tree, src: &[u8]) -> Vec<String> {
        common::captures(&self.imports_query, tree, src)
            .into_iter()
            .map(|(_, path)| path.trim_matches(['"', '`']).to_owned())
            .collect()
    }

    fn resolve_import(&self, from: &RelPath, spec: &str, layout: &ProjectLayout) -> Vec<RelPath> {
        crate::resolve::go::resolve(from, spec, layout)
    }

    fn tags(&self, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Tag> {
        common::tags(&self.tags_query, path, tree, src)
    }
}

struct SymbolWalker<'a> {
    src: &'a [u8],
    sink: SymbolSink<'a>,
}

impl SymbolWalker<'_> {
    fn source_file(&mut self, root: Node) {
        for node in named_children(root) {
            match node.kind() {
                "function_declaration" => self.function(node, SymbolKind::Function, None),
                "method_declaration" => {
                    let receiver = receiver_type(node, self.src);
                    self.function(node, SymbolKind::Method, receiver);
                }
                "type_declaration" => self.grouped(node, "type", Self::type_spec),
                "const_declaration" => self.grouped(node, "const", Self::const_spec),
                _ => {}
            }
        }
    }

    fn function(&mut self, node: Node, kind: SymbolKind, receiver: Option<String>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let name = text(name, self.src).into_owned();
        let sig_end = node
            .child_by_field_name("body")
            .map_or(node.end_byte(), |b| b.start_byte());
        let signature = signature_between(self.src, node.start_byte(), sig_end);
        let range = self.attached_start(node)..node.end_byte();
        if let Some(receiver) = &receiver {
            self.sink.enter(receiver.clone());
        }
        self.sink.push(range, kind, &name, signature);
        if receiver.is_some() {
            self.sink.leave();
        }
    }

    /// `type X ...` atau `type ( A ...; B ... )`, begitu juga `const`.
    /// Satu spec tanpa kurung: rentang mencakup kata kunci. Berkelompok:
    /// rentang per spec beserta doc comment-nya di dalam kurung.
    fn grouped(&mut self, decl: Node, keyword: &str, each: fn(&mut Self, Node, Node, &str)) {
        let grouped = has_paren_group(decl, self.src);
        for spec in named_children(decl) {
            if spec.kind() == "comment" {
                continue;
            }
            let outer = if grouped { spec } else { decl };
            each(self, spec, outer, keyword);
        }
    }

    fn type_spec(&mut self, spec: Node, outer: Node, keyword: &str) {
        let Some(ty) = spec.child_by_field_name("type") else {
            return;
        };
        let kind = match ty.kind() {
            "struct_type" => SymbolKind::Struct,
            "interface_type" => SymbolKind::Interface,
            _ => return,
        };
        let Some(name) = spec.child_by_field_name("name") else {
            return;
        };
        let name = text(name, self.src).into_owned();
        // Signature sampai sebelum `{`: `type Session struct`.
        let brace = self
            .src
            .get(ty.byte_range())
            .and_then(|b| b.iter().position(|&c| c == b'{'))
            .map_or(ty.end_byte(), |p| ty.start_byte() + p);
        self.push(spec, outer, keyword, kind, &name, brace);
    }

    fn const_spec(&mut self, spec: Node, outer: Node, keyword: &str) {
        let sig_end = spec
            .child_by_field_name("value")
            .map_or(spec.end_byte(), |v| v.start_byte());
        let mut cursor = spec.walk();
        // Field `name` juga memuat token `,` di antara nama.
        let names: Vec<Node> = spec
            .children_by_field_name("name", &mut cursor)
            .filter(|n| n.is_named())
            .collect();
        for name in names {
            let name = text(name, self.src).into_owned();
            self.push(spec, outer, keyword, SymbolKind::Const, &name, sig_end);
        }
    }

    fn push(
        &mut self,
        spec: Node,
        outer: Node,
        keyword: &str,
        kind: SymbolKind,
        name: &str,
        sig_end: usize,
    ) {
        let range = self.attached_start(outer)..outer.end_byte();
        // Spec dalam kelompok tidak memuat kata kunci; tambahkan agar
        // signature selalu terbaca utuh.
        let signature = signature_between(self.src, spec.start_byte(), sig_end)
            .map(|s| format!("{keyword} {s}"));
        self.sink.push(range, kind, name, signature);
    }

    fn attached_start(&self, node: Node) -> usize {
        attached_start(node, self.src, |kind, _| kind == "comment")
    }
}

/// Deklarasi berbentuk `type ( ... )` atau `const ( ... )`.
fn has_paren_group(decl: Node, src: &[u8]) -> bool {
    let mut cursor = decl.walk();
    decl.children(&mut cursor)
        .any(|c| !c.is_named() && text(c, src) == "(")
}

/// Nama tipe receiver: `(s *Session[T])` menjadi `Session`.
fn receiver_type(method: Node, src: &[u8]) -> Option<String> {
    let params = method.child_by_field_name("receiver")?;
    let param = named_children(params)
        .into_iter()
        .find(|n| n.kind() == "parameter_declaration")?;
    let mut ty = param.child_by_field_name("type")?;
    loop {
        ty = match ty.kind() {
            "pointer_type" => named_children(ty).into_iter().next()?,
            "generic_type" => ty.child_by_field_name("type")?,
            _ => return Some(text(ty, src).into_owned()),
        };
    }
}
