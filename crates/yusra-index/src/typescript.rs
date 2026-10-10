use tree_sitter::{Language, Node, Query, Tree};
use yusra_core::{RelPath, Symbol, SymbolKind, Tag, YusraError};

use crate::{
    LanguageAdapter,
    common::{self, SymbolSink, attached_start, signature_between, text},
};

/// Spesifikasi modul dari `import`, `export ... from`, `require()`, dan `import()`.
const IMPORTS_QUERY: &str = r#"
(import_statement source: (string (string_fragment) @import))
(export_statement source: (string (string_fragment) @import))
((call_expression
   function: (identifier) @_fn
   arguments: (arguments . (string (string_fragment) @import)))
 (#eq? @_fn "require"))
(call_expression
  function: (import)
  arguments: (arguments . (string (string_fragment) @import)))
"#;

#[derive(Clone, Copy)]
enum Dialect {
    TypeScript,
    Tsx,
    JavaScript,
}

/// TypeScript, TSX, dan JavaScript. Grammar TypeScript adalah perluasan
/// grammar JavaScript, jadi ketiganya berbagi aturan ekstraksi yang sama.
///
/// Path simbol: `Session`, `Session::refresh`, `auth::login` untuk namespace,
/// `default` untuk ekspor default tanpa nama. Konstanta hanya dari `const`;
/// `let`/`var` hanya dicatat jika nilainya fungsi.
pub struct TypeScriptAdapter {
    dialect: Dialect,
    language: Language,
    tags_query: Query,
    imports_query: Query,
}

impl TypeScriptAdapter {
    pub fn typescript() -> Result<Self, YusraError> {
        Self::new(Dialect::TypeScript)
    }

    pub fn tsx() -> Result<Self, YusraError> {
        Self::new(Dialect::Tsx)
    }

    pub fn javascript() -> Result<Self, YusraError> {
        Self::new(Dialect::JavaScript)
    }

    fn new(dialect: Dialect) -> Result<Self, YusraError> {
        let language: Language = match dialect {
            Dialect::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Dialect::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Dialect::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        };
        // Query tag TypeScript hanya berisi tambahan di atas query JavaScript.
        let tags_source = match dialect {
            Dialect::JavaScript => tree_sitter_javascript::TAGS_QUERY.to_owned(),
            Dialect::TypeScript | Dialect::Tsx => format!(
                "{}\n{}",
                tree_sitter_javascript::TAGS_QUERY,
                tree_sitter_typescript::TAGS_QUERY
            ),
        };
        let id = dialect.id();
        let tags_query = Query::new(&language, &tags_source)
            .map_err(|e| YusraError::internal_with(format!("query tags {id} tidak valid"), e))?;
        let imports_query = Query::new(&language, IMPORTS_QUERY)
            .map_err(|e| YusraError::internal_with(format!("query impor {id} tidak valid"), e))?;
        Ok(Self {
            dialect,
            language,
            tags_query,
            imports_query,
        })
    }
}

impl Dialect {
    const fn id(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
        }
    }
}

impl LanguageAdapter for TypeScriptAdapter {
    fn id(&self) -> &'static str {
        self.dialect.id()
    }

    fn extensions(&self) -> &'static [&'static str] {
        match self.dialect {
            Dialect::TypeScript => &["ts", "mts", "cts"],
            Dialect::Tsx => &["tsx"],
            Dialect::JavaScript => &["js", "mjs", "cjs", "jsx"],
        }
    }

    fn language(&self) -> Language {
        self.language.clone()
    }

    fn symbols(&self, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Symbol> {
        let mut walker = SymbolWalker {
            src,
            sink: SymbolSink::new(path, src),
        };
        walker.statements(tree.root_node());
        walker.sink.finish()
    }

    fn imports(&self, tree: &Tree, src: &[u8]) -> Vec<String> {
        common::captures(&self.imports_query, tree, src)
            .into_iter()
            .map(|(_, text)| text)
            .collect()
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
    fn statements(&mut self, parent: Node) {
        for node in named_children(parent) {
            self.statement(node, node);
        }
    }

    /// `outer` adalah node terluar (pembungkus `export` atau `declare`) yang
    /// menentukan awal rentang dan signature; `node` adalah deklarasinya.
    fn statement(&mut self, node: Node, outer: Node) {
        match node.kind() {
            "export_statement" => {
                if let Some(decl) = node.child_by_field_name("declaration") {
                    self.statement(decl, outer);
                } else if let Some(value) = node.child_by_field_name("value") {
                    self.default_export(value, outer);
                }
            }
            "ambient_declaration" => {
                for child in named_children(node) {
                    self.statement(child, outer);
                }
            }
            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                self.push_named(node, outer, SymbolKind::Function);
            }
            "class_declaration" | "abstract_class_declaration" => {
                if let Some(name) = self.push_named(node, outer, SymbolKind::Class) {
                    self.class_members(node, name);
                }
            }
            "interface_declaration" => {
                self.push_named(node, outer, SymbolKind::Interface);
            }
            "enum_declaration" => {
                self.push_named(node, outer, SymbolKind::Enum);
            }
            "internal_module" | "module" => {
                if let Some(name) = self.push_named(node, outer, SymbolKind::Module)
                    && let Some(body) = node.child_by_field_name("body")
                {
                    self.sink.enter(name);
                    self.statements(body);
                    self.sink.leave();
                }
            }
            "lexical_declaration" | "variable_declaration" => self.variables(node, outer),
            _ => {}
        }
    }

    /// `export default <ekspresi>`: hanya fungsi dan kelas yang dicatat.
    fn default_export(&mut self, value: Node, outer: Node) {
        let kind = match value.kind() {
            "arrow_function" | "function_expression" | "generator_function" => SymbolKind::Function,
            "class" => SymbolKind::Class,
            _ => return,
        };
        self.push(value, outer, kind, "default");
        if kind == SymbolKind::Class {
            self.class_members(value, "default".to_owned());
        }
    }

    fn class_members(&mut self, class: Node, name: String) {
        let Some(body) = class.child_by_field_name("body") else {
            return;
        };
        self.sink.enter(name);
        for member in named_children(body) {
            let is_method = match member.kind() {
                "method_definition" | "method_signature" | "abstract_method_signature" => true,
                // `handle = () => {}` di komponen kelas
                "public_field_definition" | "field_definition" => {
                    member.child_by_field_name("value").is_some_and(is_function)
                }
                _ => false,
            };
            if is_method {
                self.push_named(member, member, SymbolKind::Method);
            }
        }
        self.sink.leave();
    }

    fn variables(&mut self, decl: Node, outer: Node) {
        let is_const = decl
            .child_by_field_name("kind")
            .is_some_and(|k| text(k, self.src) == "const");
        let declarators: Vec<Node> = named_children(decl)
            .into_iter()
            .filter(|n| n.kind() == "variable_declarator")
            .collect();
        // Satu deklarator: rentang mencakup kata kunci dan `export`.
        // Beberapa (`const a = 1, b = 2`): rentang per deklarator.
        let single = declarators.len() == 1;
        for d in declarators {
            let Some(name) = d.child_by_field_name("name") else {
                continue;
            };
            // Destructuring tidak punya satu nama.
            if name.kind() != "identifier" {
                continue;
            }
            let value = d.child_by_field_name("value");
            if value.is_some_and(|v| is_require(v, self.src)) {
                continue;
            }
            let kind = if value.is_some_and(is_function) {
                SymbolKind::Function
            } else if is_const {
                SymbolKind::Const
            } else {
                continue;
            };
            let name = text(name, self.src).into_owned();
            let (start, sig_start, end) = if single {
                (
                    self.attached_start(outer),
                    sig_start(outer),
                    outer.end_byte(),
                )
            } else {
                (d.start_byte(), d.start_byte(), d.end_byte())
            };
            // Fungsi: sampai sebelum isinya. Konstanta: sampai sebelum nilainya.
            let sig_end = match value {
                Some(v) if kind == SymbolKind::Function => v
                    .child_by_field_name("body")
                    .map_or(v.start_byte(), |b| b.start_byte()),
                Some(v) => v.start_byte(),
                None => end,
            };
            let signature = signature_between(self.src, sig_start, sig_end);
            self.sink.push(start..end, kind, &name, signature);
        }
    }

    fn push_named(&mut self, node: Node, outer: Node, kind: SymbolKind) -> Option<String> {
        let name = text(node.child_by_field_name("name")?, self.src);
        let name = name.trim_matches(['"', '\'']).to_owned();
        self.push(node, outer, kind, &name);
        Some(name)
    }

    fn push(&mut self, node: Node, outer: Node, kind: SymbolKind, name: &str) {
        let range = self.attached_start(outer)..outer.end_byte();
        // Field kelas berisi fungsi tidak punya `body`; pakai isi fungsinya.
        let body = node.child_by_field_name("body").or_else(|| {
            node.child_by_field_name("value")
                .and_then(|v| v.child_by_field_name("body"))
        });
        let sig_end = body.map_or(node.end_byte(), |b| b.start_byte());
        let signature = signature_between(self.src, sig_start(outer), sig_end);
        self.sink.push(range, kind, name, signature);
    }

    fn attached_start(&self, node: Node) -> usize {
        attached_start(node, self.src, is_jsdoc_or_decorator)
    }
}

/// Awal signature. Jika node diawali decorator, signature dimulai dari anak
/// pertama setelahnya agar argumen decorator tidak ikut; kata kunci `export`
/// di belakang decorator ikut terbuang.
fn sig_start(outer: Node) -> usize {
    let children = named_children(outer);
    if children.first().is_none_or(|c| c.kind() != "decorator") {
        return outer.start_byte();
    }
    children
        .iter()
        .find(|c| !matches!(c.kind(), "decorator" | "comment"))
        .map_or(outer.start_byte(), |c| c.start_byte())
}

fn is_function(node: Node) -> bool {
    matches!(
        node.kind(),
        "arrow_function" | "function_expression" | "generator_function"
    )
}

/// `require("x")`: impor CommonJS, sudah dicatat di `imports`, bukan simbol.
fn is_require(node: Node, src: &[u8]) -> bool {
    node.kind() == "call_expression"
        && node
            .child_by_field_name("function")
            .is_some_and(|f| f.kind() == "identifier" && text(f, src) == "require")
}

/// JSDoc (`/** */`) dan decorator. Komentar `//` dan `/* */` biasa tidak.
fn is_jsdoc_or_decorator(kind: &str, body: &[u8]) -> bool {
    match kind {
        "decorator" => true,
        "comment" => body.starts_with(b"/**") && !body.starts_with(b"/***") && body != b"/**/",
        _ => false,
    }
}

fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}
