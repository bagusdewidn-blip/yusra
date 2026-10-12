//! Tabel `audit_log`: satu baris per panggilan tool, berhasil atau gagal.

use rusqlite::params;
use yusra_core::YusraError;

use crate::store::{Store, db_err};

/// `args_json` tidak boleh memuat isi kode: hanya path, simbol, hash, dan
/// ukuran.
#[derive(Clone, Copy, Debug)]
pub struct AuditEntry<'a> {
    /// Milidetik Unix UTC.
    pub ts: i64,
    pub client_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub tool: &'a str,
    pub args_json: &'a str,
    /// `ok` atau kode error (`ErrorCode::as_str`).
    pub outcome: &'a str,
}

impl Store {
    pub fn append_audit(&self, e: &AuditEntry<'_>) -> Result<(), YusraError> {
        self.write(|tx| {
            tx.prepare_cached(
                "INSERT INTO audit_log (ts, client_id, session_id, tool, args_json, outcome)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .and_then(|mut stmt| {
                stmt.execute(params![
                    e.ts,
                    e.client_id,
                    e.session_id,
                    e.tool,
                    e.args_json,
                    e.outcome
                ])
            })
            .map_err(db_err("gagal menulis audit_log"))?;
            Ok(())
        })
    }

    /// Jumlah baris audit. Untuk uji dan diagnosis.
    pub fn audit_count(&self) -> Result<u64, YusraError> {
        let n: i64 =
            self.read(|c| c.query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0)))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_count() {
        let dir = match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => panic!("{e}"),
        };
        let store = match Store::open(&dir.path().join("state.db")) {
            Ok(s) => s,
            Err(e) => panic!("{e}"),
        };
        let entry = AuditEntry {
            ts: 1,
            client_id: Some("stdio"),
            session_id: None,
            tool: "read_file",
            args_json: r#"{"path":"src/lib.rs"}"#,
            outcome: "ok",
        };
        for _ in 0..3 {
            if let Err(e) = store.append_audit(&entry) {
                panic!("{e}");
            }
        }
        match store.audit_count() {
            Ok(n) => assert_eq!(n, 3),
            Err(e) => panic!("{e}"),
        }
    }
}
