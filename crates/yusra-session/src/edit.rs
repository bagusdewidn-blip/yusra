//! Validasi dan penerapan edit pada isi file di memori.
//!
//! Modul ini tidak membaca atau menulis apa pun: pemanggil memberi isi file
//! menurut session dan menyimpan hasilnya sendiri. Pemeriksaan lock juga di
//! luar modul ini (lihat [`crate::lock_covers`]).

use std::sync::Arc;

use yusra_core::{Hash32, RelPath, Symbol, SymbolPath, YusraError};
use yusra_index::{LanguageAdapter, ParsedFile, hash_bytes, index_bytes};

/// Direktori yang tidak boleh diubah lewat session.
const INTERNAL_DIRS: [&str; 2] = [".git", ".yusra"];

/// Isi satu file menurut session beserta hasil parse-nya.
#[derive(Debug)]
pub struct FileView {
    path: RelPath,
    bytes: Option<Vec<u8>>,
    /// `None` jika file tidak ada, bahasanya tidak dikenal, atau terlalu besar.
    parsed: Option<ParsedFile>,
}

impl FileView {
    pub fn path(&self) -> &RelPath {
        &self.path
    }

    /// `None` jika file tidak ada.
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    pub fn into_bytes(self) -> Option<Vec<u8>> {
        self.bytes
    }

    pub fn hash(&self) -> Option<Hash32> {
        self.bytes.as_deref().map(hash_bytes)
    }

    /// Simbol dalam urutan sumber; kosong jika file tidak diparse.
    pub fn symbols(&self) -> &[Symbol] {
        self.parsed.as_ref().map_or(&[], |p| &p.symbols)
    }

    pub fn has_errors(&self) -> bool {
        self.parsed.as_ref().is_some_and(|p| p.has_errors)
    }

    /// Simbol `symbol_path`; `not_found` untuk file atau simbol yang tidak ada.
    pub fn symbol(&self, symbol_path: &SymbolPath) -> Result<&Symbol, YusraError> {
        if self.bytes.is_none() {
            return Err(not_found_file(&self.path));
        }
        self.symbols()
            .iter()
            .find(|s| &s.symbol_path == symbol_path)
            .ok_or_else(|| YusraError::NotFound {
                kind: "simbol",
                id: format!("{}::{symbol_path}", self.path),
            })
    }
}

/// Hasil edit yang lolos validasi, siap disimpan pemanggil.
#[derive(Debug)]
pub struct EditedFile {
    /// Isi file baru; `bytes()` `None` jika file dihapus.
    pub view: FileView,
    /// Hash isi simbol (atau file) sebelum edit; `None` untuk file baru.
    pub base_hash: Option<Hash32>,
    /// Hash isi simbol (atau file) sesudah edit; untuk file yang dihapus,
    /// hash isi kosong.
    pub new_hash: Hash32,
    /// Simbol yang diedit di versi baru, jika masih bisa ditemukan.
    pub symbol: Option<Symbol>,
}

/// Membuat [`FileView`] dan menerapkan edit dengan adapter bahasa yang sama.
pub struct Editor {
    adapters: Arc<[Box<dyn LanguageAdapter>]>,
    max_file_bytes: u64,
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor")
            .field("max_file_bytes", &self.max_file_bytes)
            .finish_non_exhaustive()
    }
}

impl Editor {
    /// File yang lebih besar dari `max_file_bytes` tidak diparse, jadi tidak
    /// bisa diedit per simbol.
    pub fn new(adapters: Arc<[Box<dyn LanguageAdapter>]>, max_file_bytes: u64) -> Self {
        Self {
            adapters,
            max_file_bytes,
        }
    }

    /// `bytes` `None` berarti file tidak ada menurut session.
    pub fn view(&self, path: &RelPath, bytes: Option<Vec<u8>>) -> Result<FileView, YusraError> {
        let parsed = match &bytes {
            Some(b) => index_bytes(path, b, &self.adapters, self.max_file_bytes)?.parsed,
            None => None,
        };
        Ok(FileView {
            path: path.clone(),
            bytes,
            parsed,
        })
    }

    /// Ganti isi satu simbol (termasuk doc comment dan atribut yang menempel)
    /// dengan `new_source`. `expected` adalah `body_hash` simbol yang dibaca
    /// agen; jika berbeda, edit ditolak sebagai `stale_base`.
    pub fn edit_symbol(
        &self,
        view: &FileView,
        symbol_path: &SymbolPath,
        expected: Hash32,
        new_source: &str,
    ) -> Result<EditedFile, YusraError> {
        let path = &view.path;
        check_path(path)?;
        if symbol_path.is_whole_file() {
            return Err(rejected(
                path,
                "gunakan edit seluruh file untuk mengganti seluruh file",
            ));
        }
        let Some(bytes) = view.bytes() else {
            return Err(not_found_file(path));
        };
        if view.parsed.is_none() {
            return Err(rejected(
                path,
                "bahasa file tidak didukung atau file terlalu besar untuk edit per simbol",
            ));
        }
        let symbol = view.symbol(symbol_path)?;
        if symbol.body_hash != expected {
            return Err(YusraError::StaleBase {
                path: path.to_string(),
                symbol_path: Some(symbol_path.to_string()),
                expected,
                actual: Some(symbol.body_hash),
            });
        }

        let range = symbol.byte_range.clone();
        let (Some(before), Some(after)) = (bytes.get(..range.start), bytes.get(range.end..)) else {
            return Err(YusraError::internal(format!(
                "rentang simbol {symbol_path} di luar isi {path}"
            )));
        };
        let mut new_bytes = Vec::with_capacity(before.len() + new_source.len() + after.len());
        new_bytes.extend_from_slice(before);
        new_bytes.extend_from_slice(new_source.as_bytes());
        new_bytes.extend_from_slice(after);

        let new_view = self.view(path, Some(new_bytes))?;
        reject_new_errors(view, &new_view)?;
        let new_symbol = new_view
            .symbols()
            .iter()
            .find(|s| &s.symbol_path == symbol_path)
            .or_else(|| {
                new_view
                    .symbols()
                    .iter()
                    .find(|s| s.byte_range.start == range.start)
            })
            .cloned();
        Ok(EditedFile {
            view: new_view,
            base_hash: Some(expected),
            new_hash: hash_bytes(new_source.as_bytes()),
            symbol: new_symbol,
        })
    }

    /// Tulis seluruh isi file (`content` `None` untuk menghapus). `expected`
    /// adalah hash file yang dibaca agen, atau `None` untuk membuat file baru.
    pub fn replace_file(
        &self,
        view: &FileView,
        expected: Option<Hash32>,
        content: Option<&[u8]>,
    ) -> Result<EditedFile, YusraError> {
        let path = &view.path;
        check_path(path)?;
        let actual = view.hash();
        if actual != expected {
            return Err(match expected {
                Some(expected) => YusraError::StaleBase {
                    path: path.to_string(),
                    symbol_path: None,
                    expected,
                    actual,
                },
                None => rejected(path, "file sudah ada; sertakan hash-nya untuk menimpa"),
            });
        }
        if content.is_none() && actual.is_none() {
            return Err(not_found_file(path));
        }
        let new_view = self.view(path, content.map(<[u8]>::to_vec))?;
        reject_new_errors(view, &new_view)?;
        Ok(EditedFile {
            view: new_view,
            base_hash: actual,
            new_hash: hash_bytes(content.unwrap_or_default()),
            symbol: None,
        })
    }
}

/// Tolak path di direktori internal (`.git` di mana pun, `.yusra` di root).
pub fn check_path(path: &RelPath) -> Result<(), YusraError> {
    let first = path.components().next().unwrap_or_default();
    if INTERNAL_DIRS.contains(&first) || path.components().any(|c| c == ".git") {
        return Err(YusraError::PathDenied {
            path: path.to_string(),
        });
    }
    Ok(())
}

/// Galat sintaks baru ditolak; file yang memang sudah rusak tetap boleh
/// diedit agar bisa diperbaiki bertahap.
fn reject_new_errors(before: &FileView, after: &FileView) -> Result<(), YusraError> {
    if after.has_errors() && !before.has_errors() {
        return Err(YusraError::ParseError {
            path: after.path.to_string(),
        });
    }
    Ok(())
}

fn rejected(path: &RelPath, reason: &str) -> YusraError {
    YusraError::PatchRejected {
        path: path.to_string(),
        reason: reason.to_owned(),
    }
}

fn not_found_file(path: &RelPath) -> YusraError {
    YusraError::NotFound {
        kind: "file",
        id: path.to_string(),
    }
}
