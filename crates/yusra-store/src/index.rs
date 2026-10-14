//! Tabel index: `files`, `symbols`, `file_imports`, `file_deps`,
//! `tag_defs`, `tag_refs`, dan identitas indexer di `index_meta`.

use std::{collections::HashMap, ops::Range};

use rusqlite::{OptionalExtension, Row, params};
use yusra_core::{Hash32, RelPath, Symbol, SymbolKind, SymbolPath, YusraError};

use crate::store::{Store, db_err};

/// Kunci `index_meta` untuk identitas indexer yang menghasilkan index.
const FINGERPRINT_KEY: &str = "indexer";

/// Satu definisi untuk repo map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagDef {
    pub name: String,
    /// Dimulai dari 1.
    pub line: u32,
    /// Teks baris definisi, sudah dirapikan pemanggil.
    pub text: String,
}

/// Bahan repo map satu file seperti disimpan: definisi dalam urutan sumber
/// dan jumlah kemunculan tiap nama rujukan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileTagRows {
    pub defs: Vec<TagDef>,
    /// Nama unik, jumlah > 0.
    pub refs: Vec<(String, u32)>,
}

/// Satu file hasil indexing, siap disimpan. Simbol, impor, dependensi, dan
/// tag lama file yang sama diganti seluruhnya.
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
    /// Spesifikasi impor mentah dari adapter, dalam urutan sumber.
    pub imports: &'a [String],
    /// File lain yang diimpor file ini. Tidak harus sudah terindeks.
    pub deps: &'a [RelPath],
    pub tags: &'a FileTagRows,
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
            let mut clear_defs = tx
                .prepare_cached("DELETE FROM tag_defs WHERE path = ?1")
                .map_err(db_err("gagal menyiapkan kueri tag_defs"))?;
            let mut clear_refs = tx
                .prepare_cached("DELETE FROM tag_refs WHERE path = ?1")
                .map_err(db_err("gagal menyiapkan kueri tag_refs"))?;
            let mut insert_def = tx
                .prepare_cached(
                    "INSERT INTO tag_defs (path, seq, name, line, text) VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(db_err("gagal menyiapkan kueri tag_defs"))?;
            let mut insert_ref = tx
                .prepare_cached("INSERT INTO tag_refs (path, name, count) VALUES (?1, ?2, ?3)")
                .map_err(db_err("gagal menyiapkan kueri tag_refs"))?;
            let mut clear_imports = tx
                .prepare_cached("DELETE FROM file_imports WHERE path = ?1")
                .map_err(db_err("gagal menyiapkan kueri file_imports"))?;
            let mut insert_import = tx
                .prepare_cached("INSERT INTO file_imports (path, seq, spec) VALUES (?1, ?2, ?3)")
                .map_err(db_err("gagal menyiapkan kueri file_imports"))?;
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
                clear_imports
                    .execute([path])
                    .map_err(db_err("gagal menghapus impor lama"))?;
                for (seq, spec) in rec.imports.iter().enumerate() {
                    insert_import
                        .execute(params![path, seq as i64, spec])
                        .map_err(db_err("gagal menyimpan impor"))?;
                }
                for dep in rec.deps {
                    insert_dep
                        .execute([path, dep.as_str()])
                        .map_err(db_err("gagal menyimpan dependensi"))?;
                }
                clear_defs
                    .execute([path])
                    .map_err(db_err("gagal menghapus tag lama"))?;
                clear_refs
                    .execute([path])
                    .map_err(db_err("gagal menghapus tag lama"))?;
                for (seq, def) in rec.tags.defs.iter().enumerate() {
                    insert_def
                        .execute(params![path, seq as i64, def.name, def.line, def.text])
                        .map_err(db_err("gagal menyimpan definisi tag"))?;
                }
                for (name, count) in &rec.tags.refs {
                    insert_ref
                        .execute(params![path, name, count])
                        .map_err(db_err("gagal menyimpan rujukan tag"))?;
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

    /// Tag semua file terindeks. Setiap file di `files` punya entri, juga
    /// yang tanpa tag, sehingga ketiadaan entri berarti file belum terindeks.
    pub fn all_tags(&self) -> Result<HashMap<RelPath, FileTagRows>, YusraError> {
        type Rows = (
            Vec<String>,
            Vec<(String, String, u32, String)>,
            Vec<(String, String, u32)>,
        );
        let (files, defs, refs): Rows = self.read(|c| {
            let files = c
                .prepare("SELECT path FROM files")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            let defs = c
                .prepare("SELECT path, name, line, text FROM tag_defs ORDER BY path, seq")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let refs = c
                .prepare("SELECT path, name, count FROM tag_refs ORDER BY path, name")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((files, defs, refs))
        })?;
        let mut out = HashMap::with_capacity(files.len());
        for p in files {
            out.insert(rel_path_from_db(&p)?, FileTagRows::default());
        }
        for (p, name, line, text) in defs {
            if let Some(tags) = out.get_mut(&rel_path_from_db(&p)?) {
                tags.defs.push(TagDef { name, line, text });
            }
        }
        for (p, name, count) in refs {
            if let Some(tags) = out.get_mut(&rel_path_from_db(&p)?) {
                tags.refs.push((name, count));
            }
        }
        Ok(out)
    }

    /// Spesifikasi impor semua file terindeks, dalam urutan sumber. Seperti
    /// [`Store::all_tags`], setiap file punya entri, juga yang tanpa impor.
    pub fn all_imports(&self) -> Result<HashMap<RelPath, Vec<String>>, YusraError> {
        let (files, specs): (Vec<String>, Vec<(String, String)>) = self.read(|c| {
            let files = c
                .prepare("SELECT path FROM files")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            let specs = c
                .prepare("SELECT path, spec FROM file_imports ORDER BY path, seq")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((files, specs))
        })?;
        let mut out = HashMap::with_capacity(files.len());
        for p in files {
            out.insert(rel_path_from_db(&p)?, Vec::new());
        }
        for (p, spec) in specs {
            if let Some(list) = out.get_mut(&rel_path_from_db(&p)?) {
                list.push(spec);
            }
        }
        Ok(out)
    }

    /// Semua sisi `file_deps` (pengimpor, yang diimpor), urut.
    pub fn all_deps(&self) -> Result<Vec<(RelPath, RelPath)>, YusraError> {
        let rows: Vec<(String, String)> = self.read(|c| {
            c.prepare("SELECT from_path, to_path FROM file_deps")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })?;
        let mut out = rows
            .iter()
            .map(|(from, to)| Ok((rel_path_from_db(from)?, rel_path_from_db(to)?)))
            .collect::<Result<Vec<_>, YusraError>>()?;
        out.sort_unstable();
        Ok(out)
    }

    /// Hapus sisi `removed` lalu tambahkan sisi `added` di `file_deps`, dalam
    /// satu transaksi. Pengimpor di `added` harus sudah terindeks.
    pub fn update_deps<'a>(
        &self,
        removed: impl IntoIterator<Item = (&'a RelPath, &'a RelPath)>,
        added: impl IntoIterator<Item = (&'a RelPath, &'a RelPath)>,
    ) -> Result<(), YusraError> {
        self.write(|tx| {
            let mut delete = tx
                .prepare_cached("DELETE FROM file_deps WHERE from_path = ?1 AND to_path = ?2")
                .map_err(db_err("gagal menyiapkan kueri file_deps"))?;
            let mut insert = tx
                .prepare_cached(
                    "INSERT OR IGNORE INTO file_deps (from_path, to_path) VALUES (?1, ?2)",
                )
                .map_err(db_err("gagal menyiapkan kueri file_deps"))?;
            for (from, to) in removed {
                delete
                    .execute([from.as_str(), to.as_str()])
                    .map_err(db_err("gagal menghapus dependensi"))?;
            }
            for (from, to) in added {
                insert
                    .execute([from.as_str(), to.as_str()])
                    .map_err(db_err("gagal menyimpan dependensi"))?;
            }
            Ok(())
        })
    }

    /// Identitas indexer yang menghasilkan index tersimpan, `None` jika belum
    /// pernah diatur.
    pub fn index_fingerprint(&self) -> Result<Option<String>, YusraError> {
        self.read(|c| {
            c.query_row(
                "SELECT value FROM index_meta WHERE key = ?1",
                [FINGERPRINT_KEY],
                |r| r.get(0),
            )
            .optional()
        })
    }

    /// Kosongkan seluruh index (file, simbol, impor, dependensi, tag) dan
    /// catat `fingerprint` sebagai identitas indexer baru, dalam satu transaksi.
    pub fn reset_index(&self, fingerprint: &str) -> Result<(), YusraError> {
        self.write(|tx| {
            for table in [
                "tag_refs",
                "tag_defs",
                "file_deps",
                "file_imports",
                "symbols",
                "files",
            ] {
                tx.execute(&format!("DELETE FROM {table}"), [])
                    .map_err(db_err("gagal mengosongkan index"))?;
            }
            tx.execute(
                "INSERT INTO index_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [FINGERPRINT_KEY, fingerprint],
            )
            .map_err(db_err("gagal menyimpan identitas indexer"))?;
            Ok(())
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
    /// huruf besar-kecil ASCII), opsional disaring `kind`. Urut relevansi
    /// (segmen terakhir sama persis termasuk huruf, lalu sama tanpa beda huruf,
    /// lalu ada segmen berawalan `query`, lalu substring biasa), kemudian path
    /// dan posisi; paling banyak `limit` hasil.
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
                 ORDER BY CASE
                     WHEN symbol_path = ?1
                       OR substr(symbol_path, -length(?1) - 2) = '::' || ?1
                       THEN 0
                     WHEN lower(symbol_path) = lower(?1)
                       OR substr(lower(symbol_path), -length(?1) - 2) = '::' || lower(?1)
                       THEN 1
                     WHEN instr(lower(symbol_path), lower(?1)) = 1
                       OR instr(lower(symbol_path), '::' || lower(?1)) > 0
                       THEN 2
                     ELSE 3
                 END, path, start_byte, symbol_path
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
            imports: &[],
            deps,
            tags: &NO_TAGS,
        }
    }

    static NO_TAGS: FileTagRows = FileTagRows {
        defs: Vec::new(),
        refs: Vec::new(),
    };

    fn def(name: &str, line: u32) -> TagDef {
        TagDef {
            name: name.to_owned(),
            line,
            text: format!("fn {name}()"),
        }
    }

    #[test]
    fn tags_round_trip_replace_and_cascade() {
        let (_dir, store) = temp_store();
        let a = rel("src/a.rs");
        let b = rel("src/b.rs");
        let tags_a = FileTagRows {
            defs: vec![def("zeta", 1), def("alpha", 2)],
            refs: vec![("alpha".to_owned(), 3), ("other".to_owned(), 1)],
        };
        ok(store.replace_files(&[
            FileRecord {
                tags: &tags_a,
                ..record(&a, 1, &[], &[])
            },
            record(&b, 2, &[], &[]),
        ]));
        let all = ok(store.all_tags());
        assert_eq!(all.len(), 2);
        // Urutan definisi mengikuti sumber, bukan nama.
        assert_eq!(all.get(&a), Some(&tags_a));
        // File tanpa tag tetap punya entri.
        assert_eq!(all.get(&b), Some(&FileTagRows::default()));

        let tags_a2 = FileTagRows {
            defs: vec![def("beta", 5)],
            refs: vec![],
        };
        ok(store.replace_file(&FileRecord {
            tags: &tags_a2,
            ..record(&a, 3, &[], &[])
        }));
        assert_eq!(ok(store.all_tags()).get(&a), Some(&tags_a2));

        ok(store.remove_files(std::slice::from_ref(&a)));
        let all = ok(store.all_tags());
        assert_eq!(all.len(), 1);
        assert!(!all.contains_key(&a));
        let left: i64 = match store.read(|c| {
            c.query_row(
                "SELECT (SELECT count(*) FROM tag_defs) + (SELECT count(*) FROM tag_refs)",
                [],
                |r| r.get(0),
            )
        }) {
            Ok(n) => n,
            Err(e) => panic!("{e}"),
        };
        assert_eq!(left, 0, "tag ikut terhapus bersama file");
    }

    #[test]
    fn imports_round_trip_replace_and_cascade() {
        let (_dir, store) = temp_store();
        let a = rel("src/a.rs");
        let b = rel("src/b.rs");
        let specs_a = ["crate::z".to_owned(), "crate::b".to_owned()];
        ok(store.replace_files(&[
            FileRecord {
                imports: &specs_a,
                ..record(&a, 1, &[], &[])
            },
            record(&b, 2, &[], &[]),
        ]));
        let all = ok(store.all_imports());
        assert_eq!(all.len(), 2);
        // Urutan sumber, bukan urutan abjad.
        assert_eq!(all.get(&a).map(Vec::as_slice), Some(&specs_a[..]));
        assert_eq!(all.get(&b).map(Vec::as_slice), Some(&[][..]));

        let specs_a2 = ["crate::c".to_owned()];
        ok(store.replace_file(&FileRecord {
            imports: &specs_a2,
            ..record(&a, 3, &[], &[])
        }));
        assert_eq!(
            ok(store.all_imports()).get(&a).map(Vec::as_slice),
            Some(&specs_a2[..])
        );

        ok(store.remove_files(std::slice::from_ref(&a)));
        let left: i64 = match store
            .read(|c| c.query_row("SELECT count(*) FROM file_imports", [], |r| r.get(0)))
        {
            Ok(n) => n,
            Err(e) => panic!("{e}"),
        };
        assert_eq!(left, 0, "impor ikut terhapus bersama file");
    }

    #[test]
    fn update_deps_removes_then_adds() {
        let (_dir, store) = temp_store();
        let a = rel("a.rs");
        let b = rel("b.rs");
        let c = rel("c.rs");
        ok(store.replace_files(&[
            record(&b, 2, &[], std::slice::from_ref(&c)),
            record(&a, 1, &[], std::slice::from_ref(&c)),
        ]));
        assert_eq!(
            ok(store.all_deps()),
            [(a.clone(), c.clone()), (b.clone(), c.clone())]
        );

        ok(store.update_deps([(&a, &c)], [(&a, &b), (&b, &a), (&b, &c)]));
        assert_eq!(ok(store.deps_of(&a)), std::slice::from_ref(&b));
        assert_eq!(ok(store.deps_of(&b)), [a.clone(), c.clone()]);

        // Sisi yang tidak ada diabaikan.
        ok(store.update_deps([(&a, &b), (&b, &a), (&c, &a)], []));
        assert_eq!(ok(store.all_deps()), [(b.clone(), c.clone())]);
    }

    #[test]
    fn reset_index_clears_everything_and_records_fingerprint() {
        let (_dir, store) = temp_store();
        assert_eq!(ok(store.index_fingerprint()), None);
        let a = rel("src/a.rs");
        let syms = [sym("src/a.rs", "f", SymbolKind::Function, 0..5, 1)];
        let tags = FileTagRows {
            defs: vec![def("f", 1)],
            refs: vec![("g".to_owned(), 1)],
        };
        let specs = ["crate::g".to_owned()];
        ok(store.replace_file(&FileRecord {
            tags: &tags,
            imports: &specs,
            ..record(&a, 1, &syms, std::slice::from_ref(&a))
        }));

        ok(store.reset_index("v1"));
        assert!(ok(store.all_imports()).is_empty());
        assert_eq!(ok(store.index_fingerprint()).as_deref(), Some("v1"));
        assert!(ok(store.file_hashes()).is_empty());
        assert!(ok(store.all_tags()).is_empty());
        assert!(ok(store.search_symbols("f", None, 50)).is_empty());

        ok(store.reset_index("v2"));
        assert_eq!(ok(store.index_fingerprint()).as_deref(), Some("v2"));
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

    #[test]
    fn search_ranks_exact_then_prefix_before_path() {
        let (_dir, store) = temp_store();
        let a = rel("src/a.rs");
        let z = rel("src/z.rs");
        let sa = [
            sym("src/a.rs", "refresh_token", SymbolKind::Function, 0..10, 1),
            sym("src/a.rs", "TokenStore", SymbolKind::Struct, 11..20, 2),
        ];
        let sz = [
            sym("src/z.rs", "Auth", SymbolKind::Struct, 0..10, 3),
            sym("src/z.rs", "Auth::token", SymbolKind::Method, 11..20, 4),
        ];
        ok(store.replace_files(&[record(&a, 1, &sa, &[]), record(&z, 2, &sz, &[])]));

        let names =
            |v: Vec<Symbol>| -> Vec<String> { v.into_iter().map(|s| s.symbol_path.0).collect() };
        assert_eq!(
            names(ok(store.search_symbols("TOKEN", None, 50))),
            ["Auth::token", "TokenStore", "refresh_token"]
        );
        assert_eq!(
            names(ok(store.search_symbols("token", None, 1))),
            ["Auth::token"]
        );
        // Huruf yang sama persis mendahului yang hanya sama tanpa beda huruf.
        assert_eq!(
            names(ok(store.search_symbols("auth", None, 50))),
            ["Auth", "Auth::token"]
        );
        let sb = [sym("src/b.rs", "auth", SymbolKind::Function, 0..10, 5)];
        ok(store.replace_files(&[record(&rel("src/b.rs"), 3, &sb, &[])]));
        assert_eq!(
            names(ok(store.search_symbols("Auth", None, 2))),
            ["Auth", "auth"]
        );
    }
}
