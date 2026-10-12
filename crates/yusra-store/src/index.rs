//! Tabel index: `files`, `symbols`, `file_deps`.

use std::{collections::HashMap, ops::Range};

use rusqlite::{OptionalExtension, Row, params};
use yusra_core::{Hash32, RelPath, Symbol, SymbolKind, SymbolPath, YusraError};

use crate::store::{Store, db_err};

/// Satu file hasil indexing, siap disimpan. Simbol dan dependensi lama file
/// yang sama diganti seluruhnya.
#[derive(Clone, Copy, Debug)]
pub struct FileRecord<'a> {
    pub path: &'a RelPath,
    pub hash: Hash32,
    /// `LanguageAdapter::id`, `None` jika bahasanya tidak dikenal.
    pub language: Option<&'a str>,
    /// Milidetik Unix UTC.
    pub indexed_at: i64,
    /// Semua harus ber-`path` sama dengan `path` di atas.
    pub symbols: &'a [Symbol],
    /// File lain yang diimpor file ini. Tidak harus sudah terindeks.
    pub deps: &'a [RelPath],
}

impl Store {
    /// Simpan banyak file dalam satu transaksi. Jika satu gagal, tidak ada
    /// yang tersimpan.
    pub fn replace_files(&self, records: &[FileRecord<'_>]) -> Result<(), YusraError> {
        self.write(|tx| {
            let mut upsert_file = tx
                .prepare_cached(
                    "INSERT INTO files (path, blake3, language, indexed_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (path) DO UPDATE SET
                       blake3 = excluded.blake3,
                       language = excluded.language,
                       indexed_at = excluded.indexed_at",
                )
                .map_err(db_err("gagal menyiapkan kueri files"))?;
            let mut clear_symbols = tx
                .prepare_cached("DELETE FROM symbols WHERE path = ?1")
                .map_err(db_err("gagal menyiapkan kueri symbols"))?;
            let mut clear_deps = tx
                .prepare_cached("DELETE FROM file_deps WHERE from_path = ?1")
                .map_err(db_err("gagal menyiapkan kueri file_deps"))?;
            let mut insert_symbol = tx
                .prepare_cached(
                    "INSERT INTO symbols
                       (path, symbol_path, kind, start_byte, end_byte, signature, body_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .map_err(db_err("gagal menyiapkan kueri symbols"))?;
            let mut insert_dep = tx
                .prepare_cached(
                    "INSERT OR IGNORE INTO file_deps (from_path, to_path) VALUES (?1, ?2)",
                )
                .map_err(db_err("gagal menyiapkan kueri file_deps"))?;

            for rec in records {
                let path = rec.path.as_str();
                upsert_file
                    .execute(params![
                        path,
                        rec.hash.as_bytes(),
                        rec.language,
                        rec.indexed_at
                    ])
                    .map_err(db_err("gagal menyimpan file"))?;
                clear_symbols
                    .execute([path])
                    .map_err(db_err("gagal menghapus simbol lama"))?;
                clear_deps
                    .execute([path])
                    .map_err(db_err("gagal menghapus dependensi lama"))?;
                for sym in rec.symbols {
                    if sym.path != *rec.path {
                        return Err(YusraError::internal(format!(
                            "simbol {} milik {}, bukan {}",
                            sym.symbol_path, sym.path, rec.path
                        )));
                    }
                    let (start, end) = byte_range_to_sql(&sym.byte_range)?;
                    insert_symbol
                        .execute(params![
                            path,
                            sym.symbol_path.as_str(),
                            sym.kind.as_str(),
                            start,
                            end,
                            sym.signature,
                            sym.body_hash.as_bytes(),
                        ])
                        .map_err(db_err("gagal menyimpan simbol"))?;
                }
                for dep in rec.deps {
                    insert_dep
                        .execute([path, dep.as_str()])
                        .map_err(db_err("gagal menyimpan dependensi"))?;
                }
            }
            Ok(())
        })
    }

    pub fn replace_file(&self, record: &FileRecord<'_>) -> Result<(), YusraError> {
        self.replace_files(std::slice::from_ref(record))
    }

    /// Hapus file dari index beserta simbol dan dependensinya. Path yang
    /// tidak terindeks diabaikan. Mengembalikan jumlah file yang terhapus.
    pub fn remove_files(&self, paths: &[RelPath]) -> Result<usize, YusraError> {
        self.write(|tx| {
            let mut stmt = tx
                .prepare_cached("DELETE FROM files WHERE path = ?1")
                .map_err(db_err("gagal menyiapkan kueri files"))?;
            let mut removed = 0;
            for p in paths {
                removed += stmt
                    .execute([p.as_str()])
                    .map_err(db_err("gagal menghapus file"))?;
            }
            Ok(removed)
        })
    }

    /// Hash semua file terindeks, untuk membandingkan dengan disk saat start.
    pub fn file_hashes(&self) -> Result<HashMap<RelPath, Hash32>, YusraError> {
        let rows: Vec<(String, [u8; 32])> = self.read(|c| {
            let mut stmt = c.prepare("SELECT path, blake3 FROM files")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })?;
        rows.into_iter()
            .map(|(p, h)| Ok((rel_path_from_db(&p)?, Hash32(h))))
            .collect()
    }

    pub fn file_hash(&self, path: &RelPath) -> Result<Option<Hash32>, YusraError> {
        let hash: Option<[u8; 32]> = self.read(|c| {
            c.query_row(
                "SELECT blake3 FROM files WHERE path = ?1",
                [path.as_str()],
                |r| r.get(0),
            )
            .optional()
        })?;
        Ok(hash.map(Hash32))
    }

    /// Simbol satu file, urut posisi di sumber.
    pub fn symbols_in_file(&self, path: &RelPath) -> Result<Vec<Symbol>, YusraError> {
        let rows = self.read(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {SYMBOL_COLUMNS} FROM symbols WHERE path = ?1
                 ORDER BY start_byte, end_byte DESC, symbol_path"
            ))?;
            stmt.query_map([path.as_str()], SymbolRow::from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(SymbolRow::into_symbol).collect()
    }

    /// Cari simbol yang `symbol_path`-nya memuat `query` (tanpa membedakan
    /// huruf besar-kecil ASCII), opsional disaring `kind`. Urut path lalu
    /// posisi; paling banyak `limit` hasil.
    pub fn search_symbols(
        &self,
        query: &str,
        kind: Option<SymbolKind>,
        limit: u32,
    ) -> Result<Vec<Symbol>, YusraError> {
        let rows = self.read(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {SYMBOL_COLUMNS} FROM symbols
                 WHERE instr(lower(symbol_path), lower(?1)) > 0
                   AND (?2 IS NULL OR kind = ?2)
                 ORDER BY path, start_byte, symbol_path
                 LIMIT ?3"
            ))?;
            stmt.query_map(
                params![query, kind.map(SymbolKind::as_str), limit],
                SymbolRow::from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(SymbolRow::into_symbol).collect()
    }

    /// File yang diimpor `path` menurut index.
    pub fn deps_of(&self, path: &RelPath) -> Result<Vec<RelPath>, YusraError> {
        let rows: Vec<String> = self.read(|c| {
            let mut stmt =
                c.prepare("SELECT to_path FROM file_deps WHERE from_path = ?1 ORDER BY to_path")?;
            stmt.query_map([path.as_str()], |r| r.get(0))?.collect()
        })?;
        rows.iter().map(|p| rel_path_from_db(p)).collect()
    }
}

const SYMBOL_COLUMNS: &str = "path, symbol_path, kind, start_byte, end_byte, signature, body_hash";

/// Baris mentah; validasi dilakukan di luar closure rusqlite agar error
/// domain tidak harus dibungkus sebagai `rusqlite::Error`.
struct SymbolRow {
    path: String,
    symbol_path: String,
    kind: String,
    start: i64,
    end: i64,
    signature: Option<String>,
    body_hash: [u8; 32],
}

impl SymbolRow {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            path: r.get(0)?,
            symbol_path: r.get(1)?,
            kind: r.get(2)?,
            start: r.get(3)?,
            end: r.get(4)?,
            signature: r.get(5)?,
            body_hash: r.get(6)?,
        })
    }

    fn into_symbol(self) -> Result<Symbol, YusraError> {
        let kind = SymbolKind::parse(&self.kind)
            .map_err(|e| YusraError::internal_with("isi kolom symbols.kind rusak", e))?;
        let byte_range = byte_range_from_sql(self.start, self.end)?;
        Ok(Symbol {
            path: rel_path_from_db(&self.path)?,
            symbol_path: SymbolPath(self.symbol_path),
            kind,
            byte_range,
            signature: self.signature,
            body_hash: Hash32(self.body_hash),
        })
    }
}

fn rel_path_from_db(raw: &str) -> Result<RelPath, YusraError> {
    RelPath::parse(raw).map_err(|e| YusraError::internal_with("path di database rusak", e))
}

fn byte_range_to_sql(r: &Range<usize>) -> Result<(i64, i64), YusraError> {
    match (i64::try_from(r.start), i64::try_from(r.end)) {
        (Ok(s), Ok(e)) => Ok((s, e)),
        _ => Err(YusraError::internal("rentang byte simbol terlalu besar")),
    }
}

fn byte_range_from_sql(start: i64, end: i64) -> Result<Range<usize>, YusraError> {
    match (usize::try_from(start), usize::try_from(end)) {
        (Ok(s), Ok(e)) if s <= e => Ok(s..e),
        _ => Err(YusraError::internal(format!(
            "rentang byte di database rusak: {start}..{end}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => panic!("{e}"),
        };
        match Store::open(&dir.path().join("state.db")) {
            Ok(s) => (dir, s),
            Err(e) => panic!("{e}"),
        }
    }

    fn ok<T>(r: Result<T, YusraError>) -> T {
        match r {
            Ok(v) => v,
            Err(e) => panic!("{e}"),
        }
    }

    fn rel(p: &str) -> RelPath {
        match RelPath::parse(p) {
            Ok(p) => p,
            Err(e) => panic!("{e}"),
        }
    }

    fn sym(path: &str, name: &str, kind: SymbolKind, range: Range<usize>, seed: u8) -> Symbol {
        Symbol {
            path: rel(path),
            symbol_path: SymbolPath(name.to_owned()),
            kind,
            byte_range: range,
            signature: Some(format!("fn {name}()")),
            body_hash: Hash32([seed; 32]),
        }
    }

    fn record<'a>(
        path: &'a RelPath,
        seed: u8,
        symbols: &'a [Symbol],
        deps: &'a [RelPath],
    ) -> FileRecord<'a> {
        FileRecord {
            path,
            hash: Hash32([seed; 32]),
            language: Some("rust"),
            indexed_at: 1_700_000_000_000,
            symbols,
            deps,
        }
    }

    #[test]
    fn replace_and_read_back() {
        let (_dir, store) = temp_store();
        let path = rel("src/auth.rs");
        let symbols = [
            sym("src/auth.rs", "Session", SymbolKind::Struct, 0..40, 1),
            sym("src/auth.rs", "impl Session", SymbolKind::Impl, 42..200, 2),
            sym(
                "src/auth.rs",
                "Session::refresh",
                SymbolKind::Method,
                60..190,
                3,
            ),
        ];
        let deps = [rel("src/token.rs"), rel("src/db.rs")];
        ok(store.replace_file(&record(&path, 9, &symbols, &deps)));

        assert_eq!(ok(store.file_hash(&path)), Some(Hash32([9; 32])));
        assert_eq!(ok(store.file_hash(&rel("src/none.rs"))), None);
        assert_eq!(ok(store.symbols_in_file(&path)), symbols);
        assert_eq!(
            ok(store.deps_of(&path)),
            [rel("src/db.rs"), rel("src/token.rs")]
        );
        let hashes = ok(store.file_hashes());
        assert_eq!(hashes.len(), 1);
        assert_eq!(hashes.get(&path), Some(&Hash32([9; 32])));
    }

    #[test]
    fn replace_drops_old_symbols_and_deps() {
        let (_dir, store) = temp_store();
        let path = rel("a.rs");
        let old = [
            sym("a.rs", "old", SymbolKind::Function, 0..10, 1),
            sym("a.rs", "kept", SymbolKind::Function, 11..20, 2),
        ];
        ok(store.replace_file(&record(&path, 1, &old, &[rel("b.rs")])));
        let new = [sym("a.rs", "kept", SymbolKind::Function, 0..9, 3)];
        ok(store.replace_file(&record(&path, 2, &new, &[])));

        assert_eq!(ok(store.symbols_in_file(&path)), new);
        assert!(ok(store.deps_of(&path)).is_empty());
        assert_eq!(ok(store.file_hash(&path)), Some(Hash32([2; 32])));
    }

    #[test]
    fn remove_cascades() {
        let (_dir, store) = temp_store();
        let a = rel("a.rs");
        let b = rel("b.rs");
        let sa = [sym("a.rs", "f", SymbolKind::Function, 0..5, 1)];
        let sb = [sym("b.rs", "g", SymbolKind::Function, 0..5, 2)];
        ok(store.replace_files(&[
            record(&a, 1, &sa, std::slice::from_ref(&b)),
            record(&b, 2, &sb, &[]),
        ]));

        assert_eq!(ok(store.remove_files(&[a.clone(), rel("missing.rs")])), 1);
        assert!(ok(store.symbols_in_file(&a)).is_empty());
        assert!(ok(store.deps_of(&a)).is_empty());
        assert_eq!(ok(store.symbols_in_file(&b)), sb);
        assert_eq!(ok(store.file_hashes()).len(), 1);
    }

    #[test]
    fn foreign_symbol_aborts_whole_batch() {
        let (_dir, store) = temp_store();
        let a = rel("a.rs");
        let b = rel("b.rs");
        let good = [sym("a.rs", "f", SymbolKind::Function, 0..5, 1)];
        let bad = [sym("a.rs", "g", SymbolKind::Function, 0..5, 2)];
        let r = store.replace_files(&[record(&a, 1, &good, &[]), record(&b, 2, &bad, &[])]);
        assert!(r.is_err());
        assert!(ok(store.file_hashes()).is_empty());
    }

    #[test]
    fn duplicate_symbol_path_is_rejected() {
        let (_dir, store) = temp_store();
        let a = rel("a.rs");
        let dup = [
            sym("a.rs", "f", SymbolKind::Function, 0..5, 1),
            sym("a.rs", "f", SymbolKind::Function, 6..9, 2),
        ];
        assert!(store.replace_file(&record(&a, 1, &dup, &[])).is_err());
        assert!(ok(store.file_hashes()).is_empty());
    }

    #[test]
    fn search_filters_and_limits() {
        let (_dir, store) = temp_store();
        let a = rel("src/a.rs");
        let b = rel("src/b.rs");
        let sa = [
            sym("src/a.rs", "Session", SymbolKind::Struct, 0..10, 1),
            sym(
                "src/a.rs",
                "Session::refresh",
                SymbolKind::Method,
                20..30,
                2,
            ),
        ];
        let sb = [
            sym("src/b.rs", "refresh_token", SymbolKind::Function, 0..10, 3),
            sym("src/b.rs", "other", SymbolKind::Function, 11..20, 4),
        ];
        ok(store.replace_files(&[record(&a, 1, &sa, &[]), record(&b, 2, &sb, &[])]));

        let names =
            |v: Vec<Symbol>| -> Vec<String> { v.into_iter().map(|s| s.symbol_path.0).collect() };
        assert_eq!(
            names(ok(store.search_symbols("REFRESH", None, 50))),
            ["Session::refresh", "refresh_token"]
        );
        assert_eq!(
            names(ok(store.search_symbols(
                "refresh",
                Some(SymbolKind::Function),
                50
            ))),
            ["refresh_token"]
        );
        assert_eq!(names(ok(store.search_symbols("", None, 3))).len(), 3);
        // `%` dan `_` bukan wildcard.
        assert!(ok(store.search_symbols("%", None, 50)).is_empty());
        assert_eq!(
            names(ok(store.search_symbols("h_t", None, 50))),
            ["refresh_token"]
        );
    }
}
