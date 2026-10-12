-- Skema awal. PRAGMA journal_mode dan foreign_keys diatur per koneksi di
-- src/store.rs karena tidak berlaku di dalam transaksi migrasi.
-- Migrasi yang sudah dirilis tidak boleh diubah; tambahkan file baru.

CREATE TABLE files (
  path        TEXT PRIMARY KEY,
  blake3      BLOB NOT NULL,
  language    TEXT,
  indexed_at  INTEGER NOT NULL
);

CREATE TABLE symbols (
  id          INTEGER PRIMARY KEY,
  path        TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
  symbol_path TEXT NOT NULL,
  kind        TEXT NOT NULL,
  start_byte  INTEGER NOT NULL,
  end_byte    INTEGER NOT NULL,
  signature   TEXT,
  body_hash   BLOB NOT NULL,
  UNIQUE (path, symbol_path)
);

CREATE TABLE file_deps (
  from_path   TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
  to_path     TEXT NOT NULL,
  PRIMARY KEY (from_path, to_path)
);

CREATE TABLE sessions (
  id            TEXT PRIMARY KEY,
  client_id     TEXT NOT NULL,
  title         TEXT NOT NULL,
  base_snapshot TEXT NOT NULL,
  head_tree     TEXT,
  status        TEXT NOT NULL CHECK (status IN
                ('active','stalled','in_review','merged','abandoned')),
  last_seen_at  INTEGER NOT NULL,
  created_at    INTEGER NOT NULL
);

CREATE TABLE session_files (
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  path        TEXT NOT NULL,
  blob_oid    TEXT,            -- NULL = file dihapus di session ini
  PRIMARY KEY (session_id, path)
);

CREATE TABLE locks (
  path        TEXT NOT NULL,
  symbol_path TEXT NOT NULL,
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  expires_at  INTEGER NOT NULL,
  PRIMARY KEY (path, symbol_path)
);

CREATE TABLE edits (
  id          INTEGER PRIMARY KEY,
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  path        TEXT NOT NULL,
  symbol_path TEXT,
  base_hash   BLOB,
  new_hash    BLOB NOT NULL,
  created_at  INTEGER NOT NULL
);

CREATE TABLE checkpoints (
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  version     INTEGER NOT NULL,
  body_json   TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (session_id, version)
);

CREATE TABLE review_queue (
  id          INTEGER PRIMARY KEY,
  session_id  TEXT NOT NULL REFERENCES sessions(id),
  merge_tree  TEXT,
  status      TEXT NOT NULL CHECK (status IN
              ('pending','conflict','verified','approved','rejected')),
  report_json TEXT,
  created_at  INTEGER NOT NULL
);

CREATE TABLE verify_approvals (
  config_hash BLOB PRIMARY KEY,
  approved_at INTEGER NOT NULL
);

CREATE TABLE oauth_clients (
  client_id     TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  redirect_uris TEXT NOT NULL,
  approved_at   INTEGER,
  revoked_at    INTEGER
);

CREATE TABLE refresh_tokens (
  token_hash  BLOB PRIMARY KEY,
  client_id   TEXT NOT NULL REFERENCES oauth_clients(client_id),
  family_id   TEXT NOT NULL,
  expires_at  INTEGER NOT NULL,
  used_at     INTEGER
);

CREATE TABLE audit_log (
  id          INTEGER PRIMARY KEY,
  ts          INTEGER NOT NULL,
  client_id   TEXT,
  session_id  TEXT,
  tool        TEXT NOT NULL,
  args_json   TEXT NOT NULL,   -- tanpa isi kode: hanya path, simbol, hash
  outcome     TEXT NOT NULL
);

CREATE INDEX idx_locks_expiry ON locks(expires_at);
CREATE INDEX idx_edits_session ON edits(session_id);
CREATE INDEX idx_audit_ts ON audit_log(ts);
