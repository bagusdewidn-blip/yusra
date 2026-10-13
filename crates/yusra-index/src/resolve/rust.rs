//! Impor Rust: `use` dan `mod x;` ke file modul.
//!
//! Modul dipetakan dari letak file seperti aturan Cargo dan rustc: crate
//! root (`src/lib.rs`, `src/main.rs`, `src/bin/*.rs`, `tests/*.rs`, ...)
//! memiliki direktori dasar, dan modul `a::b` ada di `<dasar>/a/b.rs` atau
//! `<dasar>/a/b/mod.rs`. Atribut `#[path]` dan modul inline tidak diikuti.
//! Crate lain hanya dikenali jika paketnya ada di proyek (workspace).

use yusra_core::RelPath;

use super::{ProjectLayout, child, split_last, to_rel};

/// Direktori target Cargo yang setiap file `.rs` langsung di dalamnya
/// adalah crate root tersendiri.
const TARGET_DIRS: [&str; 4] = ["src/bin", "tests", "examples", "benches"];

pub(crate) fn resolve(from: &RelPath, spec: &str, layout: &ProjectLayout) -> Vec<RelPath> {
    let ctx = CrateContext::of(from.as_str(), layout);
    let mut out: Vec<RelPath> = expand_use(spec)
        .iter()
        .filter_map(|segments| ctx.resolve(segments, layout))
        .filter(|f| f != from.as_str())
        .filter_map(|f| to_rel(&f))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Letak sebuah file di dalam crate-nya.
struct CrateContext {
    /// Direktori dasar modul crate.
    base: String,
    /// Crate root, `None` jika tidak bisa ditentukan (misalnya modul bersama
    /// di `tests/common/` yang dipakai banyak crate uji).
    root_file: Option<String>,
    /// Path modul file ini relatif terhadap crate root.
    module: Vec<String>,
}

impl CrateContext {
    fn of(path: &str, layout: &ProjectLayout) -> Self {
        let (dir, _) = split_last(path);
        let own_root = || Self {
            base: dir.to_owned(),
            root_file: Some(path.to_owned()),
            module: Vec::new(),
        };
        let Some(package) = layout.cargo_package_of(path) else {
            return own_root();
        };
        let rel = strip_dir(path, package);

        if let Some(target) = TARGET_DIRS.iter().find(|t| is_under(rel, t)) {
            let target_dir = child(package, target);
            let inner = strip_dir(path, &target_dir);
            let Some((sub, _)) = inner.split_once('/') else {
                return own_root();
            };
            // `tests/<nama>/main.rs` adalah crate uji dengan modulnya sendiri.
            let sub_dir = child(&target_dir, sub);
            let sub_main = child(&sub_dir, "main.rs");
            if layout.contains(&sub_main) {
                return Self::with_base(path, sub_dir, Some(sub_main));
            }
            return Self::with_base(path, target_dir, None);
        }

        if is_under(rel, "src") {
            let src = child(package, "src");
            let main = child(&src, "main.rs");
            let lib = child(&src, "lib.rs");
            let root_file = if path == main || !layout.contains(&lib) {
                main
            } else {
                lib
            };
            return Self::with_base(path, src, Some(root_file));
        }
        // `build.rs` dan file lain di luar `src/`.
        own_root()
    }

    fn with_base(path: &str, base: String, root_file: Option<String>) -> Self {
        let mut module: Vec<String> = Vec::new();
        if root_file.as_deref() != Some(path) {
            let rel = strip_dir(path, &base);
            module = rel.split('/').map(str::to_owned).collect();
            if let Some(last) = module.pop() {
                let stem = last.strip_suffix(".rs").unwrap_or(&last);
                if stem != "mod" {
                    module.push(stem.to_owned());
                }
            }
        }
        Self {
            base,
            root_file,
            module,
        }
    }

    /// File modul terdalam yang ada untuk path `use` ini. Segmen terakhir
    /// bisa berupa item, bukan modul, jadi prefiks terpanjang yang dipakai.
    fn resolve(&self, segments: &[String], layout: &ProjectLayout) -> Option<String> {
        let (first, mut rest) = segments.split_first()?;
        let (base, root_file, mut module) = match first.as_str() {
            "crate" => (self.base.clone(), self.root_file.clone(), Vec::new()),
            "self" => (
                self.base.clone(),
                self.root_file.clone(),
                self.module.clone(),
            ),
            "super" => {
                let mut module = self.module.clone();
                module.pop()?;
                (self.base.clone(), self.root_file.clone(), module)
            }
            // Path seragam (edisi 2018): nama pertama adalah modul anak.
            name if module_file(layout, &self.base, &self.module, name).is_some() => {
                let mut module = self.module.clone();
                module.push(name.to_owned());
                (self.base.clone(), self.root_file.clone(), module)
            }
            name => {
                let src = child(layout.cargo_package_named(name)?, "src");
                let lib = child(&src, "lib.rs");
                if !layout.contains(&lib) {
                    return None;
                }
                (src, Some(lib), Vec::new())
            }
        };
        while let Some((next, tail)) = rest.split_first()
            && next == "super"
        {
            module.pop()?;
            rest = tail;
        }
        longest_prefix(layout, &base, root_file.as_deref(), module, rest)
    }
}

/// Coba `module + rest` dari yang terpanjang sampai `module` saja; jika
/// `module` kosong, crate root menjadi pilihan terakhir.
fn longest_prefix(
    layout: &ProjectLayout,
    base: &str,
    root_file: Option<&str>,
    module: Vec<String>,
    rest: &[String],
) -> Option<String> {
    let fixed = module.len();
    let mut full = module;
    full.extend(rest.iter().cloned());
    for len in (fixed.max(1)..=full.len()).rev() {
        let (parent, name) = full[..len].split_at(len - 1);
        if let Some(file) = module_file(layout, base, parent, &name[0]) {
            return Some(file);
        }
    }
    (fixed == 0).then(|| root_file.map(str::to_owned)).flatten()
}

/// `<base>/<parent>/<name>.rs` atau `<base>/<parent>/<name>/mod.rs`.
fn module_file(
    layout: &ProjectLayout,
    base: &str,
    parent: &[String],
    name: &str,
) -> Option<String> {
    let dir = parent.iter().fold(base.to_owned(), |acc, p| child(&acc, p));
    let module_dir = child(&dir, name);
    [format!("{module_dir}.rs"), child(&module_dir, "mod.rs")]
        .into_iter()
        .find(|c| layout.contains(c))
}

fn is_under(path: &str, dir: &str) -> bool {
    path.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

/// `path` tanpa awalan `dir/`; `dir` kosong berarti root.
fn strip_dir<'a>(path: &'a str, dir: &str) -> &'a str {
    if dir.is_empty() {
        return path;
    }
    path.strip_prefix(dir)
        .and_then(|r| r.strip_prefix('/'))
        .unwrap_or(path)
}

/// Pecah pohon `use` menjadi path tunggal: `a::{b, c::{self, d}}` menjadi
/// `a::b`, `a::c`, `a::c::d`. Alias `as` dan glob `*` dibuang.
fn expand_use(spec: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    expand_into(&[], spec, &mut out);
    out
}

fn expand_into(prefix: &[String], tree: &str, out: &mut Vec<Vec<String>>) {
    let tree = tree.trim();
    let tree = tree.strip_prefix("::").unwrap_or(tree);
    if let Some(open) = tree.find('{') {
        let mut head = prefix.to_vec();
        head.extend(segments(&tree[..open]));
        let inner = tree[open + 1..]
            .strip_suffix('}')
            .unwrap_or(&tree[open + 1..]);
        for item in split_top_level(inner) {
            match item.trim() {
                "" => {}
                "self" => out.push(head.clone()),
                item => expand_into(&head, item, out),
            }
        }
        return;
    }
    let path = tree.rsplit_once(" as ").map_or(tree, |(p, _)| p);
    let mut full = prefix.to_vec();
    full.extend(segments(path));
    if !full.is_empty() {
        out.push(full);
    }
}

fn segments(path: &str) -> impl Iterator<Item = String> + '_ {
    path.split("::")
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "*")
        .map(|s| s.strip_prefix("r#").unwrap_or(s).to_owned())
}

/// Pisah dengan `,` yang tidak berada di dalam `{}`.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(files: &[(&str, &str)]) -> ProjectLayout {
        ProjectLayout::new(files.iter().filter_map(|(f, _)| to_rel(f)), |path| {
            files
                .iter()
                .find(|(f, _)| *f == path.as_str())
                .map(|(_, content)| content.as_bytes().to_vec())
        })
    }

    fn resolved(layout: &ProjectLayout, from: &str, spec: &str) -> Vec<String> {
        let Some(from) = to_rel(from) else {
            panic!("path uji tidak valid: {from}");
        };
        resolve(&from, spec, layout)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn joined(paths: &[Vec<String>]) -> Vec<String> {
        paths.iter().map(|p| p.join("::")).collect()
    }

    #[test]
    fn expands_use_trees() {
        assert_eq!(
            joined(&expand_use("std::{fmt::{self, Display}, io as stdio, *}")),
            ["std::fmt", "std::fmt::Display", "std::io", "std"]
        );
        assert_eq!(joined(&expand_use("::core::mem")), ["core::mem"]);
        assert_eq!(joined(&expand_use("super::*")), ["super"]);
        assert_eq!(joined(&expand_use("r#type::X")), ["type::X"]);
        assert_eq!(
            joined(&expand_use("crate::{a::{b, c}, d}")),
            ["crate::a::b", "crate::a::c", "crate::d"]
        );
    }

    fn workspace() -> ProjectLayout {
        layout(&[
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
            ("crates/core/Cargo.toml", "[package]\nname = \"my-core\"\n"),
            ("crates/core/src/lib.rs", ""),
            ("crates/core/src/path.rs", ""),
            ("crates/app/Cargo.toml", "[package]\nname = \"app\"\n"),
            ("crates/app/src/main.rs", ""),
            ("crates/app/src/lib.rs", ""),
            ("crates/app/src/store.rs", ""),
            ("crates/app/src/store/txn.rs", ""),
            ("crates/app/src/net/mod.rs", ""),
            ("crates/app/src/net/http.rs", ""),
            ("crates/app/src/bin/tool.rs", ""),
            ("crates/app/src/bin/tool_args.rs", ""),
            ("crates/app/tests/common/mod.rs", ""),
            ("crates/app/tests/walk.rs", ""),
            ("crates/app/tests/big/main.rs", ""),
            ("crates/app/tests/big/part.rs", ""),
            ("crates/app/build.rs", ""),
        ])
    }

    #[test]
    fn crate_self_and_super_paths() {
        let l = workspace();
        let lib = "crates/app/src/lib.rs";
        let txn = "crates/app/src/store/txn.rs";
        let http = "crates/app/src/net/http.rs";
        assert_eq!(
            resolved(&l, lib, "self::store"),
            ["crates/app/src/store.rs"]
        );
        assert_eq!(
            resolved(&l, lib, "crate::store::txn::Txn"),
            ["crates/app/src/store/txn.rs"]
        );
        assert_eq!(
            resolved(&l, lib, "crate::store::{Value, txn}"),
            ["crates/app/src/store.rs", "crates/app/src/store/txn.rs"]
        );
        assert_eq!(
            resolved(&l, txn, "super::Value"),
            ["crates/app/src/store.rs"]
        );
        assert_eq!(
            resolved(&l, txn, "super::super::net"),
            ["crates/app/src/net/mod.rs"]
        );
        assert_eq!(
            resolved(&l, http, "self::super::X"),
            ["crates/app/src/net/mod.rs"]
        );
        assert_eq!(resolved(&l, txn, "crate::Thing"), ["crates/app/src/lib.rs"]);
        // main.rs adalah crate root sendiri walau lib.rs ada.
        assert_eq!(
            resolved(&l, "crates/app/src/main.rs", "self::store"),
            ["crates/app/src/store.rs"]
        );
        assert!(resolved(&l, lib, "super::x").is_empty());
    }

    #[test]
    fn uniform_paths_and_mod_declarations() {
        let l = workspace();
        // `mod net;` di lib.rs dicatat adapter sebagai `self::net`.
        let lib = "crates/app/src/lib.rs";
        assert_eq!(
            resolved(&l, lib, "self::net"),
            ["crates/app/src/net/mod.rs"]
        );
        assert_eq!(
            resolved(&l, lib, "net::http::get"),
            ["crates/app/src/net/http.rs"]
        );
        // `mod txn;` di store.rs (bukan mod.rs) ada di `store/txn.rs`.
        assert_eq!(
            resolved(&l, "crates/app/src/store.rs", "self::txn"),
            ["crates/app/src/store/txn.rs"]
        );
    }

    #[test]
    fn workspace_crates_and_externals() {
        let l = workspace();
        let lib = "crates/app/src/lib.rs";
        assert_eq!(
            resolved(&l, lib, "my_core::path::RelPath"),
            ["crates/core/src/path.rs"]
        );
        assert_eq!(
            resolved(&l, lib, "my_core::Hash32"),
            ["crates/core/src/lib.rs"]
        );
        assert!(resolved(&l, lib, "std::fmt").is_empty());
        assert!(resolved(&l, lib, "serde::Serialize").is_empty());
    }

    #[test]
    fn target_dirs_are_their_own_crate_roots() {
        let l = workspace();
        let test = "crates/app/tests/walk.rs";
        assert_eq!(
            resolved(&l, test, "self::common"),
            ["crates/app/tests/common/mod.rs"]
        );
        assert_eq!(
            resolved(&l, test, "crate::common::rel"),
            ["crates/app/tests/common/mod.rs"]
        );
        assert_eq!(
            resolved(&l, test, "app::store"),
            ["crates/app/src/store.rs"]
        );
        assert_eq!(
            resolved(&l, "crates/app/tests/big/main.rs", "self::part"),
            ["crates/app/tests/big/part.rs"]
        );
        assert_eq!(
            resolved(&l, "crates/app/src/bin/tool.rs", "self::tool_args"),
            ["crates/app/src/bin/tool_args.rs"]
        );
        // Modul bersama tidak punya satu crate root.
        assert!(resolved(&l, "crates/app/tests/common/mod.rs", "crate::X").is_empty());
        assert!(resolved(&l, "crates/app/build.rs", "crate::store").is_empty());
    }
}
