#![cfg(all(
    feature = "lang-rust",
    feature = "lang-typescript",
    feature = "lang-python",
    feature = "lang-go"
))]

mod common;

use std::{fs, path::Path};

use common::rel;
use yusra_core::RelPath;
use yusra_index::{
    ImportGraph, ProjectLayout, builtin_adapters, index_file, resolve_imports, walk,
};

fn write(root: &Path, path: &str, content: &str) {
    let full = root.join(path);
    if let Some(parent) = full.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        panic!("{e}");
    }
    if let Err(e) = fs::write(&full, content) {
        panic!("{e}");
    }
}

const PROJECT: &[(&str, &str)] = &[
    // Workspace Rust dengan dua crate.
    ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
    (
        "crates/core/Cargo.toml",
        "[package]\nname = \"demo-core\"\nversion = \"0.1.0\"\n",
    ),
    (
        "crates/core/src/lib.rs",
        "pub mod path;\npub use path::P;\n",
    ),
    ("crates/core/src/path.rs", "pub struct P;\n"),
    (
        "crates/app/Cargo.toml",
        "[package]\nname = \"demo-app\"\nversion = \"0.1.0\"\n",
    ),
    (
        "crates/app/src/main.rs",
        "mod store;\nuse demo_core::path::P;\nuse std::fmt;\nfn main() {}\n",
    ),
    (
        "crates/app/src/store.rs",
        "use crate::store::txn::Txn;\nmod txn;\npub struct Value;\n",
    ),
    (
        "crates/app/src/store/txn.rs",
        "use super::Value;\npub struct Txn;\n",
    ),
    // TypeScript.
    (
        "web/src/app.ts",
        "import { cfg } from './config';\nimport React from 'react';\nexport * from \"./ui\";\n",
    ),
    ("web/src/config.ts", "export const cfg = 1;\n"),
    (
        "web/src/ui/index.tsx",
        "const u = require('../config.js');\nexport const X = 1;\n",
    ),
    // Python.
    ("py/app/__init__.py", ""),
    (
        "py/app/views.py",
        "import os\nfrom .models import User\nfrom . import helpers\n",
    ),
    ("py/app/models.py", "class User: pass\n"),
    ("py/app/helpers.py", "def h(): pass\n"),
    // Go.
    ("go/go.mod", "module example.com/demo\n\ngo 1.22\n"),
    (
        "go/cmd/main.go",
        "package main\n\nimport (\n\t\"fmt\"\n\t\"example.com/demo/store\"\n)\n\nfunc main() { fmt.Println(store.X) }\n",
    ),
    ("go/store/store.go", "package store\n\nvar X = 1\n"),
    ("go/store/store_test.go", "package store\n"),
];

fn build() -> (Vec<(RelPath, Vec<String>)>, ProjectLayout) {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    let root = dir.path();
    for (path, content) in PROJECT {
        write(root, path, content);
    }
    let adapters = match builtin_adapters() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    };
    let walked = match walk(root, &[]) {
        Ok(w) => w,
        Err(e) => panic!("{e}"),
    };
    let mut imports = Vec::new();
    for entry in &walked.files {
        let indexed = match index_file(root, &entry.path, &adapters, 1 << 20) {
            Ok(f) => f,
            Err(e) => panic!("{e}"),
        };
        if let Some(parsed) = indexed.parsed {
            imports.push((indexed.path, parsed.imports));
        }
    }
    (imports, ProjectLayout::load(root, &walked.files))
}

#[test]
fn builds_graph_across_languages() {
    let adapters = match builtin_adapters() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    };
    let (imports, layout) = build();
    let graph = ImportGraph::build(
        &adapters,
        &layout,
        imports.iter().map(|(p, specs)| (p, specs.as_slice())),
    );
    let edges: Vec<String> = graph
        .edges()
        .map(|(from, to)| format!("{from} -> {to}"))
        .collect();
    assert_eq!(
        edges,
        [
            "crates/app/src/main.rs -> crates/app/src/store.rs",
            "crates/app/src/main.rs -> crates/core/src/path.rs",
            "crates/app/src/store.rs -> crates/app/src/store/txn.rs",
            "crates/app/src/store/txn.rs -> crates/app/src/store.rs",
            "crates/core/src/lib.rs -> crates/core/src/path.rs",
            "go/cmd/main.go -> go/store/store.go",
            "py/app/views.py -> py/app/helpers.py",
            "py/app/views.py -> py/app/models.py",
            "web/src/app.ts -> web/src/config.ts",
            "web/src/app.ts -> web/src/ui/index.tsx",
            "web/src/ui/index.tsx -> web/src/config.ts",
        ]
    );

    let importers: Vec<String> = graph
        .importers_of(&rel("web/src/config.ts"))
        .map(ToString::to_string)
        .collect();
    assert_eq!(importers, ["web/src/app.ts", "web/src/ui/index.tsx"]);
    assert_eq!(graph.imports_of(&rel("go/store/store.go")).count(), 0);
}

#[test]
fn unknown_language_and_manifests() {
    let adapters = match builtin_adapters() {
        Ok(a) => a,
        Err(e) => panic!("{e}"),
    };
    let (_, layout) = build();
    let specs = vec!["./config".to_owned()];
    assert!(resolve_imports(&adapters, &layout, &rel("web/src/notes.md"), &specs).is_empty());
    assert!(ProjectLayout::is_manifest(&rel("crates/app/Cargo.toml")));
    assert!(ProjectLayout::is_manifest(&rel("go/go.mod")));
    assert!(!ProjectLayout::is_manifest(&rel("go/go.sum")));
}
