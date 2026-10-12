//! Traversal file proyek dan indexing satu file.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use ignore::{
    WalkBuilder,
    overrides::{Override, OverrideBuilder},
};
use yusra_core::{Hash32, RelPath, Symbol, Tag, YusraError};

use crate::{LanguageAdapter, adapter_for, hash_bytes, parse};

/// Nama file ignore khusus yusra, sintaksnya sama dengan `.gitignore`.
pub const YUSRA_IGNORE_FILE: &str = ".yusraignore";

/// Direktori state daemon di root proyek.
const STATE_DIR: &str = ".yusra";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: RelPath,
    pub size: u64,
}

/// Entri yang dilewati traversal, untuk dilaporkan ke log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skipped {
    /// Symlink tidak pernah diikuti agar isi di luar root tidak terbaca.
    Symlink(String),
    /// Nama bukan UTF-8 atau ditolak `RelPath::parse`.
    InvalidName(String),
    Io(String),
}

#[derive(Debug, Default)]
pub struct Walk {
    /// Urut menurut path.
    pub files: Vec<FileEntry>,
    pub skipped: Vec<Skipped>,
}

/// Semua file biasa di bawah `root`, dengan aturan:
/// - `.gitignore`, `.git/info/exclude`, `.ignore`, dan `.yusraignore` dihormati,
///   tetapi tidak `.gitignore` global milik user dan tidak file di atas `root`,
///   agar hasil sama di mesin mana pun;
/// - `extra_ignore` (dari `yusra.toml`) memakai sintaks `.gitignore` dan hanya
///   bisa mengecualikan, jadi pola berawalan `!` ditolak;
/// - file tersembunyi ikut (`.github/`, `.eslintrc.js`); `.git` di mana pun dan
///   `.yusra/` di root selalu dilewati;
/// - symlink tidak diikuti.
///
/// `deny_read` tidak diterapkan di sini; itu tugas lapisan yang membaca isi
/// file untuk agen.
pub fn walk(root: &Path, extra_ignore: &[String]) -> Result<Walk, YusraError> {
    let mut out = Walk::default();
    for result in walk_builder(root, extra_ignore)?.build() {
        let entry = match result {
            Ok(e) => e,
            Err(e) => {
                out.skipped.push(Skipped::Io(e.to_string()));
                continue;
            }
        };
        let display = || display_path(root, entry.path());
        if entry.path_is_symlink() {
            out.skipped.push(Skipped::Symlink(display()));
            continue;
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(path) = rel_path(root, entry.path()) else {
            out.skipped.push(Skipped::InvalidName(display()));
            continue;
        };
        match entry.metadata() {
            Ok(m) => out.files.push(FileEntry {
                path,
                size: m.len(),
            }),
            Err(e) => out.skipped.push(Skipped::Io(e.to_string())),
        }
    }
    out.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Konfigurasi traversal bersama `walk` dan [`crate::PathFilter`], agar
/// keduanya selalu memutuskan sama.
pub(crate) fn walk_builder(
    root: &Path,
    extra_ignore: &[String],
) -> Result<WalkBuilder, YusraError> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .parents(false)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .add_custom_ignore_filename(YUSRA_IGNORE_FILE)
        .overrides(build_overrides(root, extra_ignore)?)
        .filter_entry(|e| !is_state_path(e.file_name().to_str(), e.depth()));
    Ok(builder)
}

/// `.git` di kedalaman mana pun dan `.yusra` langsung di bawah root.
/// `depth` dihitung dari root: anak langsung root berkedalaman 1.
pub(crate) fn is_state_path(name: Option<&str>, depth: usize) -> bool {
    name == Some(".git") || (depth == 1 && name == Some(STATE_DIR))
}

/// Pola `extra_ignore` sebagai override. Pola berawalan `!` ditolak.
fn build_overrides(root: &Path, extra_ignore: &[String]) -> Result<Override, YusraError> {
    let mut overrides = OverrideBuilder::new(root);
    for pattern in extra_ignore {
        if pattern.starts_with('!') {
            return Err(YusraError::internal(format!(
                "pola extra_ignore tidak boleh diawali '!': {pattern:?}"
            )));
        }
        // Di override, pola tanpa `!` berarti whitelist; `!` berarti abaikan.
        overrides.add(&format!("!{pattern}")).map_err(|e| {
            YusraError::internal_with(format!("pola extra_ignore tidak valid: {pattern:?}"), e)
        })?;
    }
    overrides
        .build()
        .map_err(|e| YusraError::internal_with("pola extra_ignore tidak valid", e))
}

/// Hasil indexing satu file.
#[derive(Debug)]
pub struct IndexedFile {
    pub path: RelPath,
    pub hash: Hash32,
    /// `LanguageAdapter::id`, `None` jika tidak ada adapter untuk ekstensinya.
    pub language: Option<&'static str>,
    /// `None` jika bahasa tidak dikenal atau file lebih besar dari batas.
    pub parsed: Option<ParsedFile>,
}

#[derive(Debug)]
pub struct ParsedFile {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<String>,
    pub tags: Vec<Tag>,
    /// Pohon memuat node `ERROR`; simbol tetap diekstrak sebisanya.
    pub has_errors: bool,
}

/// Baca, hash, dan (jika bahasanya dikenal dan ukurannya tidak melebihi
/// `max_file_bytes`) parse satu file. File yang lebih besar tetap di-hash.
/// Symlink ditolak sebagai `not_found`.
pub fn index_file(
    root: &Path,
    path: &RelPath,
    adapters: &[Box<dyn LanguageAdapter>],
    max_file_bytes: u64,
) -> Result<IndexedFile, YusraError> {
    let full = full_path(root, path);
    let not_found = || YusraError::NotFound {
        kind: "file",
        id: path.to_string(),
    };
    let meta = fs::symlink_metadata(&full).map_err(|e| io_error(e, path, not_found))?;
    if !meta.file_type().is_file() {
        return Err(not_found());
    }
    let bytes = fs::read(&full).map_err(|e| io_error(e, path, not_found))?;
    index_bytes(path, &bytes, adapters, max_file_bytes)
}

/// Seperti [`index_file`], untuk isi yang sudah dibaca pemanggil (misalnya
/// agar isi yang sama juga dipakai untuk teks baris repo map).
pub fn index_bytes(
    path: &RelPath,
    bytes: &[u8],
    adapters: &[Box<dyn LanguageAdapter>],
    max_file_bytes: u64,
) -> Result<IndexedFile, YusraError> {
    let hash = hash_bytes(bytes);
    let adapter = adapter_for(adapters, path);
    let parsed = match adapter {
        Some(a) if bytes.len() as u64 <= max_file_bytes => {
            let tree = parse(a, bytes)?;
            Some(ParsedFile {
                symbols: a.symbols(path, &tree, bytes),
                imports: a.imports(&tree, bytes),
                tags: a.tags(path, &tree, bytes),
                has_errors: tree.root_node().has_error(),
            })
        }
        _ => None,
    };
    Ok(IndexedFile {
        path: path.clone(),
        hash,
        language: adapter.map(|a| a.id()),
        parsed,
    })
}

fn io_error(e: io::Error, path: &RelPath, not_found: impl Fn() -> YusraError) -> YusraError {
    match e.kind() {
        io::ErrorKind::NotFound => not_found(),
        _ => YusraError::internal_with(format!("gagal membaca {path}"), e),
    }
}

fn full_path(root: &Path, path: &RelPath) -> PathBuf {
    let mut full = root.to_path_buf();
    full.extend(path.components());
    full
}

/// Path relatif dengan pemisah `/` di semua platform, divalidasi `RelPath`.
pub(crate) fn rel_path(root: &Path, path: &Path) -> Option<RelPath> {
    let mut parts = Vec::new();
    for c in path.strip_prefix(root).ok()?.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?),
            _ => return None,
        }
    }
    RelPath::parse(&parts.join("/")).ok()
}

fn display_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}
