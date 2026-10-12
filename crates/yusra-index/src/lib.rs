//! Indexing: traversal, tree-sitter, simbol, graf impor, watcher.

mod adapter;
#[cfg(any(
    feature = "lang-rust",
    feature = "lang-typescript",
    feature = "lang-python",
    feature = "lang-go"
))]
mod common;
mod filter;
#[cfg(feature = "lang-go")]
mod go;
#[cfg(feature = "lang-python")]
mod python;
#[cfg(feature = "lang-rust")]
mod rust;
#[cfg(feature = "lang-typescript")]
mod typescript;
mod walk;
mod watch;

pub use adapter::{LanguageAdapter, adapter_for, builtin_adapters, parse};
pub use filter::PathFilter;
#[cfg(feature = "lang-go")]
pub use go::GoAdapter;
#[cfg(feature = "lang-python")]
pub use python::PythonAdapter;
#[cfg(feature = "lang-rust")]
pub use rust::RustAdapter;
#[cfg(feature = "lang-typescript")]
pub use typescript::TypeScriptAdapter;
pub use walk::{
    FileEntry, IndexedFile, ParsedFile, Skipped, Walk, YUSRA_IGNORE_FILE, index_bytes, index_file,
    walk,
};
pub use watch::{Change, DEBOUNCE, Watcher, watch};

use yusra_core::Hash32;

/// blake3 dari `bytes`, dipakai untuk hash file dan `Symbol::body_hash`.
pub fn hash_bytes(bytes: &[u8]) -> Hash32 {
    Hash32(*blake3::hash(bytes).as_bytes())
}

/// Naikkan setiap kali kode adapter mengubah simbol atau tag untuk isi file
/// yang sama. Perubahan teks query sudah tertangkap otomatis oleh
/// [`index_fingerprint`].
const INDEX_FORMAT: u32 = 1;

/// Sumber query yang dikompilasi ke build ini, per bahasa yang aktif.
fn query_sources() -> Vec<(&'static str, &'static str)> {
    #[allow(
        unused_mut,
        reason = "setiap push bergantung pada cargo feature bahasa"
    )]
    let mut sources: Vec<(&'static str, &'static str)> = Vec::new();
    #[cfg(feature = "lang-rust")]
    sources.extend([
        ("rust/tags", rust::TAGS_QUERY),
        ("rust/imports", rust::IMPORTS_QUERY),
    ]);
    #[cfg(feature = "lang-typescript")]
    sources.extend([
        ("javascript/tags", tree_sitter_javascript::TAGS_QUERY),
        ("typescript/tags", tree_sitter_typescript::TAGS_QUERY),
        ("typescript/imports", typescript::IMPORTS_QUERY),
    ]);
    #[cfg(feature = "lang-python")]
    sources.push(("python/tags", tree_sitter_python::TAGS_QUERY));
    #[cfg(feature = "lang-go")]
    sources.extend([
        ("go/tags", tree_sitter_go::TAGS_QUERY),
        ("go/imports", go::IMPORTS_QUERY),
    ]);
    sources
}

/// Identitas hasil indexing build ini: versi crate, [`INDEX_FORMAT`], dan
/// hash semua query bahasa yang aktif. Index tersimpan dengan identitas lain
/// harus dibangun ulang karena simbol dan tagnya bisa berbeda.
pub fn index_fingerprint() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    for (name, source) in query_sources() {
        for part in [name, source] {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
    }
    let hex = hasher.finalize().to_hex();
    format!("{INDEX_FORMAT}-{}", &hex[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_covers_queries() {
        let a = index_fingerprint();
        assert_eq!(a, index_fingerprint());
        assert!(a.starts_with(&format!("{INDEX_FORMAT}-")), "{a}");
        #[cfg(feature = "lang-rust")]
        assert!(query_sources().iter().any(|(n, _)| *n == "rust/tags"));
    }
}
