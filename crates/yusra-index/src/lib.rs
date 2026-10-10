//! Indexing: traversal, tree-sitter, simbol, graf impor, watcher.

mod adapter;
#[cfg(any(
    feature = "lang-rust",
    feature = "lang-typescript",
    feature = "lang-python",
    feature = "lang-go"
))]
mod common;
#[cfg(feature = "lang-go")]
mod go;
#[cfg(feature = "lang-python")]
mod python;
#[cfg(feature = "lang-rust")]
mod rust;
#[cfg(feature = "lang-typescript")]
mod typescript;

pub use adapter::{LanguageAdapter, adapter_for, builtin_adapters, parse};
#[cfg(feature = "lang-go")]
pub use go::GoAdapter;
#[cfg(feature = "lang-python")]
pub use python::PythonAdapter;
#[cfg(feature = "lang-rust")]
pub use rust::RustAdapter;
#[cfg(feature = "lang-typescript")]
pub use typescript::TypeScriptAdapter;

use yusra_core::Hash32;

/// blake3 dari `bytes`, dipakai untuk hash file dan `Symbol::body_hash`.
pub fn hash_bytes(bytes: &[u8]) -> Hash32 {
    Hash32(*blake3::hash(bytes).as_bytes())
}
