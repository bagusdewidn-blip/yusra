use std::{
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use rusqlite_migration::{M, Migrations, SchemaVersion};
use yusra_core::YusraError;

const MIGRATIONS: &[M<'static>] = &[M::up(include_str!("../migrations/0001_init.sql"))];

/// Lama menunggu lock SQLite dari proses atau koneksi lain sebelum menyerah.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Koneksi baca menganggur yang disimpan untuk dipakai ulang. Permintaan baca
/// yang bersamaan melebihi angka ini tetap dilayani dengan koneksi baru.
const MAX_IDLE_READERS: usize = 4;

pub(crate) fn migrations() -> Migrations<'static> {
    Migrations::from_slice(MIGRATIONS)
}

/// State satu proyek (`.yusra/state.db`): satu koneksi tulis di balik mutex
/// dan kumpulan koneksi hanya-baca. Mode WAL membuat pembaca tidak menunggu
/// penulis.
///
/// Semua method sinkron; dari kode async panggil lewat `spawn_blocking`.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
}

impl Store {
    /// Buka atau buat database, lalu jalankan migrasi yang belum diterapkan.
    /// Direktori induk harus sudah ada. Database dengan skema lebih baru dari
    /// binary ini ditolak agar tidak rusak oleh versi lama.
    pub fn open(path: &Path) -> Result<Self, YusraError> {
        let mut writer = Connection::open(path).map_err(db_err("gagal membuka database"))?;
        configure_writer(&writer)?;
        migrate(&mut writer)?;
        Ok(Self {
            path: path.to_owned(),
            writer: Mutex::new(writer),
            readers: Mutex::new(Vec::new()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Versi skema (`PRAGMA user_version`), sama dengan jumlah migrasi.
    pub fn schema_version(&self) -> Result<u32, YusraError> {
        self.read(|c| c.pragma_query_value(None, "user_version", |r| r.get(0)))
    }

    /// Jalankan `f` dengan koneksi hanya-baca. Beberapa kueri di dalam `f`
    /// hanya melihat snapshot yang sama jika `f` membuka transaksi sendiri.
    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, YusraError> {
        let idle = lock(&self.readers).pop();
        let conn = match idle {
            Some(c) => c,
            None => open_reader(&self.path)?,
        };
        let out = f(&conn).map_err(db_err("kueri baca gagal"));
        let mut idle = lock(&self.readers);
        if idle.len() < MAX_IDLE_READERS {
            idle.push(conn);
        }
        out
    }

    /// Jalankan `f` dalam satu transaksi tulis (`BEGIN IMMEDIATE`). Transaksi
    /// di-commit jika `f` berhasil dan di-rollback jika gagal atau panic.
    pub(crate) fn write<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, YusraError>,
    ) -> Result<T, YusraError> {
        let mut conn = lock(&self.writer);
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err("gagal memulai transaksi"))?;
        let out = f(&tx)?;
        tx.commit().map_err(db_err("gagal commit transaksi"))?;
        Ok(out)
    }
}

/// Panic di tengah transaksi tidak merusak koneksi: `Transaction` di-rollback
/// saat di-drop. Karena itu mutex yang poisoned tetap dipakai.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn configure_writer(conn: &Connection) -> Result<(), YusraError> {
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(db_err("gagal mengatur busy_timeout"))?;
    let mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "wal", |r| r.get(0))
        .map_err(db_err("gagal mengaktifkan WAL"))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(YusraError::internal(format!(
            "database tidak bisa memakai mode WAL (mode saat ini: {mode})"
        )));
    }
    // NORMAL aman di mode WAL: commit terakhir bisa hilang saat listrik
    // padam, tetapi database tidak rusak.
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(db_err("gagal mengatur synchronous"))?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(db_err("gagal mengaktifkan foreign_keys"))?;
    Ok(())
}

fn open_reader(path: &Path) -> Result<Connection, YusraError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let conn =
        Connection::open_with_flags(path, flags).map_err(db_err("gagal membuka koneksi baca"))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(db_err("gagal mengatur busy_timeout"))?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> Result<(), YusraError> {
    let migrations = migrations();
    let current = migrations
        .current_version(conn)
        .map_err(|e| YusraError::internal_with("gagal membaca versi skema", e))?;
    if let SchemaVersion::Outside(v) = current {
        return Err(YusraError::internal(format!(
            "skema database versi {v} lebih baru dari yang didukung ({}); perbarui yusrad",
            MIGRATIONS.len()
        )));
    }
    migrations
        .to_latest(conn)
        .map_err(|e| YusraError::internal_with("migrasi skema gagal", e))
}

pub(crate) fn db_err(context: &'static str) -> impl FnOnce(rusqlite::Error) -> YusraError {
    move |e| YusraError::internal_with(context, e)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

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

    #[test]
    fn migrations_are_valid() {
        if let Err(e) = migrations().validate() {
            panic!("{e}");
        }
    }

    #[test]
    fn open_applies_schema_and_pragmas() {
        let (_dir, store) = temp_store();
        assert_eq!(ok(store.schema_version()), MIGRATIONS.len() as u32);

        let conn = lock(&store.writer);
        let mode: String = match conn.pragma_query_value(None, "journal_mode", |r| r.get(0)) {
            Ok(m) => m,
            Err(e) => panic!("{e}"),
        };
        assert_eq!(mode, "wal");
        let fk: bool = match conn.pragma_query_value(None, "foreign_keys", |r| r.get(0)) {
            Ok(v) => v,
            Err(e) => panic!("{e}"),
        };
        assert!(fk);
        drop(conn);

        let tables: Vec<String> = ok(store.read(|c| {
            let mut stmt =
                c.prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")?;
            stmt.query_map([], |r| r.get(0))?.collect()
        }));
        let expected = [
            "audit_log",
            "checkpoints",
            "edits",
            "file_deps",
            "files",
            "locks",
            "oauth_clients",
            "refresh_tokens",
            "review_queue",
            "session_files",
            "sessions",
            "symbols",
            "verify_approvals",
        ];
        assert_eq!(tables, expected);
    }

    #[test]
    fn reopen_is_idempotent() {
        let (dir, store) = temp_store();
        drop(store);
        let again = ok(Store::open(&dir.path().join("state.db")));
        assert_eq!(ok(again.schema_version()), MIGRATIONS.len() as u32);
    }

    #[test]
    fn rejects_newer_schema() {
        let (dir, store) = temp_store();
        ok(store.write(|tx| {
            tx.pragma_update(None, "user_version", 99)
                .map_err(db_err("uji"))
        }));
        drop(store);
        match Store::open(&dir.path().join("state.db")) {
            Ok(_) => panic!("skema lebih baru harus ditolak"),
            Err(e) => assert!(e.to_string().contains("99"), "{e}"),
        }
    }

    #[test]
    fn readers_are_read_only() {
        let (_dir, store) = temp_store();
        let r = store.read(|c| c.execute("DELETE FROM files", []));
        assert!(r.is_err());
    }

    #[test]
    fn failed_write_rolls_back() {
        let (_dir, store) = temp_store();
        let r: Result<(), _> = store.write(|tx| {
            tx.execute(
                "INSERT INTO verify_approvals (config_hash, approved_at) VALUES (x'00', 1)",
                [],
            )
            .map_err(db_err("uji"))?;
            Err(YusraError::internal("batal"))
        });
        assert!(r.is_err());
        let n: i64 =
            ok(store
                .read(|c| c.query_row("SELECT count(*) FROM verify_approvals", [], |r| r.get(0))));
        assert_eq!(n, 0);
    }

    #[test]
    fn concurrent_readers_see_commits() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        let writer = {
            let store = Arc::clone(&store);
            thread::spawn(move || {
                for i in 0..50_i64 {
                    ok(store.write(|tx| {
                        tx.execute(
                            "INSERT INTO verify_approvals (config_hash, approved_at) VALUES (?1, ?1)",
                            [i],
                        )
                        .map_err(db_err("uji"))
                    }));
                }
            })
        };
        let readers: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                thread::spawn(move || {
                    let mut last = 0_i64;
                    for _ in 0..50 {
                        let n: i64 = ok(store.read(|c| {
                            c.query_row("SELECT count(*) FROM verify_approvals", [], |r| r.get(0))
                        }));
                        assert!(n >= last, "jumlah baris tidak boleh mundur");
                        last = n;
                    }
                })
            })
            .collect();
        for t in readers.into_iter().chain([writer]) {
            if t.join().is_err() {
                panic!("thread uji panic");
            }
        }
        let n: i64 =
            ok(store
                .read(|c| c.query_row("SELECT count(*) FROM verify_approvals", [], |r| r.get(0))));
        assert_eq!(n, 50);
        assert!(lock(&store.readers).len() <= MAX_IDLE_READERS);
    }
}
