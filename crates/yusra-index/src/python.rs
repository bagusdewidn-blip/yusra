use tree_sitter::{Language, Node, Query, Tree};
use yusra_core::{RelPath, Symbol, SymbolKind, Tag, YusraError};

use crate::{
    LanguageAdapter,
    common::{self, SymbolSink, named_children, signature_between, text},
};

/// Path simbol Python: `login`, `Session`, `Session.refresh` ditulis
/// `Session::refresh`, kelas bersarang `Session::Meta`.
///
/// Docstring dan decorator berada di dalam node definisi, jadi otomatis ikut
/// dalam rentang. Komentar `#` di atas definisi tidak ikut. Konstanta hanya
/// assignment tingkat modul dengan nama huruf besar (`MAX_RETRY = 3`).
pub struct PythonAdapter {
    language: Language,
    tags_query: Query,
}

impl PythonAdapter {
    pub fn new() -> Result<Self, YusraError> {
        let language: Language = tree_sitter_python::LANGUAGE.into();
        let tags_query = Query::new(&language, tree_sitter_python::TAGS_QUERY)
            .map_err(|e| YusraError::internal_with("query tags Python tidak valid", e))?;
        Ok(Self {
            language,
            tags_query,
        })
    }
}

impl LanguageAdapter for PythonAdapter {
    fn id(&self) -> &'static str {
        "python"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["py", "pyi"]
    }

    fn language(&self) -> Language {
        self.language.clone()
    }

    fn symbols(&self, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Symbol> {
        let mut walker = SymbolWalker {
            src,
            sink: SymbolSink::new(path, src),
        };
        walker.module(tree.root_node());
        walker.sink.finish()
    }

    /// Nama modul bertitik. `from . import a` menjadi `.a`; `from .m import a`
    /// menjadi `.m`. Impor di dalam fungsi juga dicatat.
    fn imports(&self, tree: &Tree, src: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        collect_imports(tree.root_node(), src, &mut out);
        out
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
    fn module(&mut self, root: Node) {
        for node in named_children(root) {
            match node.kind() {
                "expression_statement" => self.constant(node),
                _ => self.definition(node, false),
            }
        }
    }

    /// `function_definition` atau `class_definition`, boleh dibungkus
    /// `decorated_definition`. `in_class` menentukan fungsi atau method.
    fn definition(&mut self, outer: Node, in_class: bool) {
        let node = match outer.kind() {
            "decorated_definition" => match outer.child_by_field_name("definition") {
                Some(d) => d,
                None => return,
            },
            _ => outer,
        };
        let kind = match node.kind() {
            "function_definition" if in_class => SymbolKind::Method,
            "function_definition" => SymbolKind::Function,
            "class_definition" => SymbolKind::Class,
            _ => return,
        };
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let name = text(name, self.src).into_owned();
        // Signature dari `def`/`class` (tanpa decorator) sampai sebelum blok.
        let sig_end = node
            .child_by_field_name("body")
            .map_or(node.end_byte(), |b| b.start_byte());
        let signature = signature_between(self.src, node.start_byte(), sig_end);
        self.sink
            .push(outer.start_byte()..outer.end_byte(), kind, &name, signature);

        if kind == SymbolKind::Class
            && let Some(body) = node.child_by_field_name("body")
        {
            self.sink.enter(name);
            for member in named_children(body) {
                self.definition(member, true);
            }
            self.sink.leave();
        }
    }

    fn constant(&mut self, stmt: Node) {
        let Some(assign) = named_children(stmt).into_iter().next() else {
            return;
        };
        if assign.kind() != "assignment" {
            return;
        }
        let Some(left) = assign.child_by_field_name("left") else {
            return;
        };
        let name = text(left, self.src);
        if left.kind() != "identifier" || !is_constant_name(&name) {
            return;
        }
        let sig_end = assign
            .child_by_field_name("right")
            .map_or(assign.end_byte(), |r| r.start_byte());
        let signature = signature_between(self.src, stmt.start_byte(), sig_end);
        self.sink.push(
            stmt.start_byte()..stmt.end_byte(),
            SymbolKind::Const,
            &name,
            signature,
        );
    }
}

/// Huruf besar, angka, dan `_`, dengan minimal satu huruf: `MAX_RETRY`, `API_V2`.
fn is_constant_name(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn collect_imports(node: Node, src: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "import_statement" => {
            for name in field_children(node, "name") {
                out.push(module_name(name, src));
            }
        }
        "import_from_statement" => {
            let Some(module) = node.child_by_field_name("module_name") else {
                return;
            };
            let module = text(module, src).into_owned();
            // `from . import a, b`: yang diimpor adalah submodul `a` dan `b`.
            if module.chars().all(|c| c == '.') {
                for name in field_children(node, "name") {
                    out.push(format!("{module}{}", module_name(name, src)));
                }
            } else {
                out.push(module);
            }
        }
        _ => {
            for child in named_children(node) {
                collect_imports(child, src, out);
            }
        }
    }
}

/// `a.b` dari `dotted_name` atau `a.b as c`.
fn module_name(node: Node, src: &[u8]) -> String {
    let name = match node.kind() {
        "aliased_import" => node.child_by_field_name("name").unwrap_or(node),
        _ => node,
    };
    text(name, src).into_owned()
}

fn field_children<'t>(node: Node<'t>, field: &str) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children_by_field_name(field, &mut cursor).collect()
}
