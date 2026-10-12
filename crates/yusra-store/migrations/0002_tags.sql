-- Tag repo map per file agar start ulang tidak perlu mem-parse file yang
-- hash-nya tidak berubah, dan identitas indexer yang menghasilkan index.

CREATE TABLE tag_defs (
  path        TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
  seq         INTEGER NOT NULL,  -- urutan dalam file
  name        TEXT NOT NULL,
  line        INTEGER NOT NULL,  -- mulai 1
  text        TEXT NOT NULL,
  PRIMARY KEY (path, seq)
);

CREATE TABLE tag_refs (
  path        TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  count       INTEGER NOT NULL CHECK (count > 0),
  PRIMARY KEY (path, name)
);

CREATE TABLE index_meta (
  key         TEXT PRIMARY KEY,
  value       TEXT NOT NULL
);

-- Index dari skema 0001 tidak punya tag; kosongkan agar dibangun ulang.
DELETE FROM file_deps;
DELETE FROM symbols;
DELETE FROM files;
