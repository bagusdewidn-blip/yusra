use std::path::PathBuf;

use crate::{
    error::YusraError,
    id::{BlobId, CommitId, SessionId, SnapshotId},
    path::RelPath,
};

/// Akses Git yang dibutuhkan daemon. Implementasi tidak boleh membuat commit di
/// branch pengguna, memindahkan `HEAD`, atau menyentuh index Git pengguna.
pub trait Vcs: Send + Sync {
    /// `None` jika repo belum punya commit.
    fn head(&self) -> Result<Option<CommitId>, YusraError>;

    /// Rekam working tree (termasuk file untracked yang tidak di-ignore) sebagai
    /// commit di `refs/yusra/snapshots/<id>` memakai index sementara.
    fn snapshot_worktree(&self, id: &SnapshotId) -> Result<CommitId, YusraError>;

    fn write_blob(&self, bytes: &[u8]) -> Result<BlobId, YusraError>;

    fn read_blob(&self, id: &BlobId) -> Result<Vec<u8>, YusraError>;

    /// `None` jika `path` tidak ada di `commit`.
    fn read_at(&self, commit: &CommitId, path: &RelPath) -> Result<Option<Vec<u8>>, YusraError>;

    fn changed_paths(&self, a: &CommitId, b: &CommitId) -> Result<Vec<RelPath>, YusraError>;

    /// Worktree fisik untuk verifikasi, dibuat hanya saat dibutuhkan.
    fn ensure_worktree(&self, session: &SessionId, at: &CommitId) -> Result<PathBuf, YusraError>;

    fn remove_worktree(&self, session: &SessionId) -> Result<(), YusraError>;
}
