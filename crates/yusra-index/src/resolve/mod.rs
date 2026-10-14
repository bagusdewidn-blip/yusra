//! Resolusi spesifikasi impor ke file proyek dan graf impor.
//!
//! Resolusi bersifat statis dan sebisanya: hanya file di dalam proyek yang
//! menjadi tujuan, impor ke pustaka luar (std, npm, PyPI, modul Go lain)
//! tidak menghasilkan apa-apa. Tidak ada konfigurasi build yang dijalankan.
#![cfg_attr(
    not(all(
        feature = "lang-rust",
        feature = "lang-typescript",
        feature = "lang-python",
        feature = "lang-go"
    )),
    allow(
        dead_code,
        reason = "helper dipakai resolver bahasa yang diaktifkan lewat cargo feature"
    )
)]

#[cfg(feature = "lang-go")]
pub(crate) mod go;
#[cfg(feature = "lang-python")]
pub(crate) mod python;
#[cfg(feature = "lang-rust")]
pub(crate) mod rust;
#[cfg(feature = "lang-typescript")]
pub(crate) mod typescript;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use yusra_core::RelPath;

use crate::{FileEntry, LanguageAdapter, adapter_for};

const CARGO_MANIFEST: &str = "Cargo.toml";
const GO_MANIFEST: &str = "go.mod";

/// Daftar file proyek beserta manifest yang dibutuhkan resolusi impor
/// (`Cargo.toml` dan `go.mod`). Bangun ulang setelah file ditambah atau
/// dihapus, atau setelah [`ProjectLayout::is_manifest`] berubah.
#[derive(Debug, Default)]
pub struct ProjectLayout {
    /// Path file dengan pemisah `/`.
    files: BTreeSet<String>,
    /// Direktori paket Cargo (`""` untuk root) ke nama crate library-nya.
    cargo_packages: BTreeMap<String, Option<String>>,
    /// Pasangan (direktori `go.mod`, path modul), modul terpanjang lebih dulu.
    go_modules: Vec<(String, String)>,
}

impl ProjectLayout {
    /// `read_manifest` dipanggil untuk setiap `Cargo.toml` dan `go.mod`;
    /// `None` berarti isinya tidak bisa dibaca dan manifest itu diabaikan
    /// sebagian (paket Cargo tetap dikenali, tetapi tanpa nama crate).
    pub fn new(
        files: impl IntoIterator<Item = RelPath>,
        mut read_manifest: impl FnMut(&RelPath) -> Option<Vec<u8>>,
    ) -> Self {
        let mut layout = Self::default();
        for path in files {
            let (dir, name) = split_last(path.as_str());
            match name {
                CARGO_MANIFEST => {
                    let crate_name = read_manifest(&path).and_then(|b| cargo_lib_name(&b));
                    layout.cargo_packages.insert(dir.to_owned(), crate_name);
                }
                GO_MANIFEST => {
                    if let Some(module) = read_manifest(&path).and_then(|b| go_module_path(&b)) {
                        layout.go_modules.push((dir.to_owned(), module));
                    }
                }
                _ => {}
            }
            layout.files.insert(path.as_str().to_owned());
        }
        layout
            .go_modules
            .sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.cmp(b)));
        layout
    }

    /// Seperti [`ProjectLayout::new`], manifest dibaca dari disk di bawah
    /// `root`. Manifest yang kini symlink atau gagal dibaca dianggap kosong.
    pub fn load(root: &Path, files: &[FileEntry]) -> Self {
        Self::new(files.iter().map(|f| f.path.clone()), |path| {
            let mut full = root.to_path_buf();
            full.extend(path.components());
            let meta = fs::symlink_metadata(&full).ok()?;
            if !meta.is_file() {
                return None;
            }
            fs::read(&full).ok()
        })
    }

    /// `true` untuk file yang isinya memengaruhi resolusi impor.
    pub fn is_manifest(path: &RelPath) -> bool {
        matches!(split_last(path.as_str()).1, CARGO_MANIFEST | GO_MANIFEST)
    }

    pub(crate) fn contains(&self, path: &str) -> bool {
        self.files.contains(path)
    }

    /// File langsung di dalam `dir`, urut menurut path.
    pub(crate) fn files_in<'a>(&'a self, dir: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let len = prefix.len();
        self.files
            .range(prefix.clone()..)
            .take_while(move |p| p.starts_with(&prefix))
            .filter(move |p| !p[len..].contains('/'))
            .map(String::as_str)
    }

    /// Direktori paket Cargo terdekat yang memuat `path`.
    pub(crate) fn cargo_package_of<'a>(&self, path: &'a str) -> Option<&'a str> {
        ancestors(path).find(|dir| self.cargo_packages.contains_key(*dir))
    }

    /// Direktori paket Cargo dengan nama crate library `name`.
    pub(crate) fn cargo_package_named(&self, name: &str) -> Option<&str> {
        self.cargo_packages
            .iter()
            .find(|(_, n)| n.as_deref() == Some(name))
            .map(|(dir, _)| dir.as_str())
    }

    pub(crate) fn go_modules(&self) -> impl Iterator<Item = (&str, &str)> {
        self.go_modules
            .iter()
            .map(|(dir, module)| (dir.as_str(), module.as_str()))
    }
}

/// Resolusi semua `specs` milik file `from` menurut adapter bahasanya.
/// Hasil tanpa `from` sendiri; kosong jika bahasanya tidak dikenal.
pub fn resolve_imports(
    adapters: &[Box<dyn LanguageAdapter>],
    layout: &ProjectLayout,
    from: &RelPath,
    specs: &[String],
) -> BTreeSet<RelPath> {
    let Some(adapter) = adapter_for(adapters, from) else {
        return BTreeSet::new();
    };
    specs
        .iter()
        .flat_map(|spec| adapter.resolve_import(from, spec, layout))
        .filter(|target| target != from)
        .collect()
}

/// Graf dependensi antar-file hasil resolusi impor. Spesifikasi impor
/// mentah per file disimpan pemanggil; karena resolusi tidak perlu parse
/// ulang, graf cukup dibangun ulang setiap kali daftar file berubah.
#[derive(Debug, Default)]
pub struct ImportGraph {
    imports: BTreeMap<RelPath, BTreeSet<RelPath>>,
    importers: BTreeMap<RelPath, BTreeSet<RelPath>>,
}

impl ImportGraph {
    pub fn build<'a>(
        adapters: &[Box<dyn LanguageAdapter>],
        layout: &ProjectLayout,
        files: impl IntoIterator<Item = (&'a RelPath, &'a [String])>,
    ) -> Self {
        let mut graph = Self::default();
        for (from, specs) in files {
            let targets = resolve_imports(adapters, layout, from, specs);
            for target in &targets {
                graph
                    .importers
                    .entry(target.clone())
                    .or_default()
                    .insert(from.clone());
            }
            if !targets.is_empty() {
                graph.imports.insert(from.clone(), targets);
            }
        }
        graph
    }

    /// Ganti sisi keluar `from` dengan `targets`, misalnya setelah isi satu
    /// file berubah tanpa daftar file atau manifest ikut berubah. `targets`
    /// kosong menghapus semua impor `from`.
    pub fn set_imports(&mut self, from: &RelPath, targets: BTreeSet<RelPath>) {
        for old in self.imports.remove(from).into_iter().flatten() {
            if let Some(set) = self.importers.get_mut(&old) {
                set.remove(from);
                if set.is_empty() {
                    self.importers.remove(&old);
                }
            }
        }
        for target in &targets {
            self.importers
                .entry(target.clone())
                .or_default()
                .insert(from.clone());
        }
        if !targets.is_empty() {
            self.imports.insert(from.clone(), targets);
        }
    }

    /// File yang diimpor `path`, urut menurut path.
    pub fn imports_of(&self, path: &RelPath) -> impl Iterator<Item = &RelPath> {
        self.imports.get(path).into_iter().flatten()
    }

    /// File yang mengimpor `path`, urut menurut path.
    pub fn importers_of(&self, path: &RelPath) -> impl Iterator<Item = &RelPath> {
        self.importers.get(path).into_iter().flatten()
    }

    /// Semua sisi `(pengimpor, yang diimpor)`, urut.
    pub fn edges(&self) -> impl Iterator<Item = (&RelPath, &RelPath)> {
        self.imports
            .iter()
            .flat_map(|(from, targets)| targets.iter().map(move |t| (from, t)))
    }
}

/// `(direktori, nama)`; direktori root ditulis `""`.
pub(crate) fn split_last(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// `dir/name`, atau `name` jika `dir` adalah root.
pub(crate) fn child(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// Gabungkan `dir` dengan path relatif bergaya POSIX (`.` dan `..` boleh).
/// `None` jika keluar dari root.
pub(crate) fn join(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// Direktori induk `path` dari yang terdekat sampai root (`""`).
pub(crate) fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    let mut next = Some(path);
    std::iter::from_fn(move || {
        let current = next?;
        let (dir, _) = split_last(current);
        next = (!current.is_empty()).then_some(dir);
        next
    })
}

pub(crate) fn to_rel(path: &str) -> Option<RelPath> {
    RelPath::parse(path).ok()
}

/// Nama crate library dari `Cargo.toml`: `[lib] name` jika ada, selain itu
/// `[package] name` dengan `-` diganti `_`. Hanya baris `kunci = "nilai"`
/// sederhana yang dibaca; itu cukup untuk dua kunci ini.
fn cargo_lib_name(manifest: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(manifest).ok()?;
    let mut section = "";
    let mut package = None;
    let mut lib = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[') {
            section = header.trim_end_matches(']').trim();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "name" {
            continue;
        }
        let value = value.split('#').next().unwrap_or_default().trim();
        let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            continue;
        };
        match section {
            "package" => package = Some(value.to_owned()),
            "lib" => lib = Some(value.to_owned()),
            _ => {}
        }
    }
    lib.or(package).map(|n| n.replace('-', "_"))
}

/// Path modul dari baris `module` di `go.mod`.
fn go_module_path(manifest: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(manifest).ok()?;
    text.lines().find_map(|line| {
        let line = line.split("//").next().unwrap_or_default().trim();
        let rest = line.strip_prefix("module")?;
        if !rest.starts_with([' ', '\t', '"']) {
            return None;
        }
        let module = rest.trim().trim_matches(['"', '`']);
        (!module.is_empty()).then(|| module.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_normalizes_and_stays_in_root() {
        assert_eq!(join("src/a", "./b").as_deref(), Some("src/a/b"));
        assert_eq!(join("src/a", "../b/./c").as_deref(), Some("src/b/c"));
        assert_eq!(join("", "x").as_deref(), Some("x"));
        assert_eq!(join("src", "..").as_deref(), Some(""));
        assert_eq!(join("src", "../.."), None);
    }

    #[test]
    fn set_imports_replaces_edges_both_ways() {
        let rel = |p: &str| match RelPath::parse(p) {
            Ok(p) => p,
            Err(e) => panic!("{e}"),
        };
        let (a, b, c) = (rel("a.rs"), rel("b.rs"), rel("c.rs"));
        let mut graph = ImportGraph::default();
        graph.set_imports(&a, [b.clone(), c.clone()].into());
        graph.set_imports(&b, [c.clone()].into());
        assert_eq!(
            graph.importers_of(&c).collect::<Vec<_>>(),
            [&a, &b],
            "dua pengimpor"
        );

        graph.set_imports(&a, [b.clone()].into());
        assert_eq!(graph.imports_of(&a).collect::<Vec<_>>(), [&b]);
        assert_eq!(graph.importers_of(&c).collect::<Vec<_>>(), [&b]);
        assert_eq!(graph.importers_of(&b).collect::<Vec<_>>(), [&a]);

        graph.set_imports(&a, BTreeSet::new());
        graph.set_imports(&b, BTreeSet::new());
        assert_eq!(graph.edges().count(), 0);
        assert!(graph.importers.is_empty(), "tanpa entri kosong tersisa");
    }

    #[test]
    fn ancestors_end_at_root() {
        let all: Vec<&str> = ancestors("a/b/c.rs").collect();
        assert_eq!(all, ["a/b", "a", ""]);
        assert_eq!(ancestors("c.rs").collect::<Vec<_>>(), [""]);
    }

    #[test]
    fn reads_cargo_lib_name() {
        let manifest = br#"
[package]
name = "yusra-core" # komentar
version = "0.1.0"

[dependencies]
name = "bukan-ini"
"#;
        assert_eq!(cargo_lib_name(manifest).as_deref(), Some("yusra_core"));
        let renamed = b"[package]\nname = \"a-b\"\n[lib]\nname = \"ab_lib\"\n";
        assert_eq!(cargo_lib_name(renamed).as_deref(), Some("ab_lib"));
        assert_eq!(cargo_lib_name(b"[workspace]\nmembers = []\n"), None);
    }

    #[test]
    fn reads_go_module_path() {
        let manifest = b"// komentar\nmodule example.com/app // akhir\n\ngo 1.22\n";
        assert_eq!(go_module_path(manifest).as_deref(), Some("example.com/app"));
        assert_eq!(
            go_module_path(b"module \"example.com/q\"\n").as_deref(),
            Some("example.com/q")
        );
        assert_eq!(go_module_path(b"modules x\n"), None);
    }
}
