# yusra'

Middleware lokal yang menghubungkan repo proyek dengan agen AI (web dan IDE) lewat Model Context Protocol, dengan isolasi per agen, lock tingkat simbol, dan persetujuan manusia sebelum kode menyentuh working tree.

Status: pra-alfa. Belum ada fitur yang bisa dipakai.

## Struktur

```text
crates/
  yusra-core/     tipe domain, error, trait bersama
  yusra-index/    tree-sitter, simbol, graf impor, watcher
  yusra-store/    SQLite, migrasi
  yusra-git/      snapshot, worktree
  yusra-merge/    merge per simbol
  yusra-session/  lock lease, edit, checkpoint, verifikasi
  yusra-auth/     OAuth 2.1, pairing
  yusra-mcp/      tool MCP, transport
  yusra-tokens/   estimasi token, repo map
  yusrad/         binary daemon dan CLI
ui/preview/       UI Preview (statis)
```

Aturan dependensi: `yusra-core` tidak bergantung pada crate lain di workspace, dan hanya `yusra-mcp` yang bergantung pada SDK MCP.

## Pengembangan

Butuh Rust 1.89 atau lebih baru.

```sh
scripts/setup-dev.sh        # Windows: scripts\setup-dev.ps1
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

`setup-dev` mengaktifkan hook di `.githooks/`. Hook `commit-msg` menghapus trailer atribusi alat AI (misalnya `Co-authored-by` dari asisten coding) dari pesan commit. CI menolak pull request yang commit atau deskripsinya masih memuat atribusi tersebut.

## Lisensi

Belum ditentukan.
