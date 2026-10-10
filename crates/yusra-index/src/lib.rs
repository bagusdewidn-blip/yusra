//! Indexing: traversal, tree-sitter, simbol, graf impor, watcher.

mod adapter;
#[cfg(feature = "lang-rust")]
mod rust;

pub use adapter::{LanguageAdapter, adapter_for, builtin_adapters, parse};
#[cfg(feature = "lang-rust")]
pub use rust::RustAdapter;

use yusra_core::Hash32;

/// blake3 dari `bytes`, dipakai untuk hash file dan `Symbol::body_hash`.
pub fn hash_bytes(bytes: &[u8]) -> Hash32 {
    Hash32(*blake3::hash(bytes).as_bytes())
}
