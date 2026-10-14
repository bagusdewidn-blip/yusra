-- Spesifikasi impor mentah per file agar graf impor bisa dibangun ulang
-- tanpa parse ketika daftar file atau manifest berubah.

CREATE TABLE file_imports (
  path        TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
  seq         INTEGER NOT NULL,  -- urutan dalam file
  spec        TEXT NOT NULL,
  PRIMARY KEY (path, seq)
);

-- Index sebelumnya tidak menyimpan impor; kosongkan agar dibangun ulang.
DELETE FROM tag_refs;
DELETE FROM tag_defs;
DELETE FROM file_deps;
DELETE FROM symbols;
DELETE FROM files;
