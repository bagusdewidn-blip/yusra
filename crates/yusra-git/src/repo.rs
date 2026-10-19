//! [`Vcs`] di atas perintah plumbing `git`.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use yusra_core::{BlobId, CommitId, RelPath, SessionId, SnapshotId, Vcs, YusraError};

use crate::cmd::Git;

/// Prefiks ref snapshot. Di luar `refs/heads` dan `refs/tags`, jadi tidak
/// muncul sebagai branch atau tag pengguna dan tidak ikut `git push` biasa.
pub const SNAPSHOT_REF_PREFIX: &str = "refs/yusra/snapshots/";

/// Direktori state daemon di root proyek; tidak pernah ikut snapshot.
const STATE_DIR: &str = ".yusra";

/// Repo Git proyek. Semua perintah memakai index sementara atau worktree
/// milik yusra: index, `HEAD`, dan branch pengguna tidak pernah diubah.
#[derive(Debug)]
pub struct GitRepo {
    root: PathBuf,
    /// Direktori Git milik worktree ini (berisi `index` dan `HEAD`).
    git_dir: PathBuf,
    /// Direktori Git bersama (berisi object dan ref); sama dengan `git_dir`
    /// kecuali `root` adalah linked worktree.
    common_dir: PathBuf,
    worktrees_dir: PathBuf,
}

impl GitRepo {
    /// `root` harus top-level working tree sebuah repo Git. Worktree
    /// verifikasi dibuat di `worktrees_dir/<session>`; sebaiknya di luar
    /// `root` agar alat build tidak membaca konfigurasi proyek di atasnya.
    pub fn open(root: &Path, worktrees_dir: &Path) -> Result<Self, YusraError> {
        let out = Git::new(root)
            .args([
                "rev-parse",
                "--show-toplevel",
                "--absolute-git-dir",
                "--git-common-dir",
            ])
            .output()?;
        if !out.status.success() {
            return Err(YusraError::NotFound {
                kind: "repo Git",
                id: root.display().to_string(),
            });
        }
        let text = String::from_utf8(out.stdout)
            .map_err(|e| YusraError::internal_with("output rev-parse bukan UTF-8", e))?;
        let mut lines = text.lines();
        let (Some(toplevel), Some(git_dir), Some(common_dir)) =
            (lines.next(), lines.next(), lines.next())
        else {
            return Err(YusraError::internal(format!(
                "output rev-parse tidak lengkap: {text:?}"
            )));
        };
        let root = canonical(root)?;
        if canonical(Path::new(toplevel))? != root {
            return Err(YusraError::internal(format!(
                "{} bukan top-level repo Git (top-level: {toplevel})",
                root.display()
            )));
        }
        // `--git-common-dir` bisa relatif terhadap direktori kerja perintah.
        let common_dir = canonical(&root.join(common_dir))?;
        Ok(Self {
            git_dir: PathBuf::from(git_dir),
            common_dir,
            root,
            worktrees_dir: worktrees_dir.to_path_buf(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Lokasi worktree verifikasi session, ada atau tidak.
    pub fn worktree_path(&self, session: &SessionId) -> PathBuf {
        self.worktrees_dir.join(session.as_str())
    }

    fn git(&self) -> Git {
        Git::new(&self.root)
    }

    fn verify_commit(&self, id: &CommitId) -> Result<(), YusraError> {
        let out = self
            .git()
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(format!("{id}^{{commit}}"))
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(YusraError::NotFound {
                kind: "commit",
                id: id.to_string(),
            })
        }
    }

    /// `true` jika `path` adalah worktree milik repo ini.
    fn owns_worktree(&self, path: &Path) -> Result<bool, YusraError> {
        if !path.join(".git").exists() {
            return Ok(false);
        }
        let out = Git::new(path)
            .args(["rev-parse", "--git-common-dir"])
            .output()?;
        if !out.status.success() {
            return Ok(false);
        }
        let common = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok(canonical(&path.join(common)).ok().as_ref() == Some(&self.common_dir))
    }
}

impl Vcs for GitRepo {
    fn head(&self) -> Result<Option<CommitId>, YusraError> {
        let out = self
            .git()
            .args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
            .output()?;
        match out.status.code() {
            Some(0) => parse_oid(&out.stdout, CommitId::parse).map(Some),
            // `--verify --quiet` keluar dengan 1 untuk HEAD yang belum lahir.
            Some(1) => Ok(None),
            _ => Err(YusraError::internal(format!(
                "rev-parse HEAD gagal: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))),
        }
    }

    fn snapshot_worktree(&self, id: &SnapshotId) -> Result<CommitId, YusraError> {
        let index = TempIndex::new(self.git_dir.join(format!("yusra-index-{id}")));
        // Salinan index pengguna membawa cache stat, sehingga `git add` hanya
        // membaca ulang file yang berubah. Index aslinya tidak disentuh.
        match fs::copy(self.git_dir.join("index"), index.path()) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(YusraError::internal_with("gagal menyalin index Git", e)),
        }
        self.git()
            .env("GIT_INDEX_FILE", index.path())
            .args(["add", "--all", "--", "."])
            .arg(format!(":(top,exclude){STATE_DIR}"))
            .run()?;
        let tree = self
            .git()
            .env("GIT_INDEX_FILE", index.path())
            .arg("write-tree")
            .run_line()?;

        let mut commit_tree = self.git().arg("commit-tree").arg(&tree);
        if let Some(head) = self.head()? {
            commit_tree = commit_tree.arg("-p").arg(head.as_str());
        }
        let commit = commit_tree
            .args(["-m", &format!("yusra snapshot {id}")])
            .run()?;
        let commit = parse_oid(&commit, CommitId::parse)?;

        // Nilai lama kosong: gagal jika ref snapshot ini sudah ada.
        self.git()
            .args(["update-ref", "-m", "yusra snapshot"])
            .arg(format!("{SNAPSHOT_REF_PREFIX}{id}"))
            .arg(commit.as_str())
            .arg("")
            .run()?;
        Ok(commit)
    }

    fn write_blob(&self, bytes: &[u8]) -> Result<BlobId, YusraError> {
        // `--no-filters`: simpan byte apa adanya, tanpa autocrlf atau filter clean.
        let out = self
            .git()
            .args(["hash-object", "-w", "--no-filters", "--stdin"])
            .stdin(bytes)
            .run()?;
        parse_oid(&out, BlobId::parse)
    }

    fn read_blob(&self, id: &BlobId) -> Result<Vec<u8>, YusraError> {
        let out = self.git().args(["cat-file", "-t", id.as_str()]).output()?;
        if !out.status.success() || out.stdout.trim_ascii() != b"blob" {
            return Err(YusraError::NotFound {
                kind: "blob",
                id: id.to_string(),
            });
        }
        self.git().args(["cat-file", "blob", id.as_str()]).run()
    }

    fn read_at(&self, commit: &CommitId, path: &RelPath) -> Result<Option<Vec<u8>>, YusraError> {
        self.verify_commit(commit)?;
        let out = self
            .git()
            .env("GIT_LITERAL_PATHSPECS", "1")
            .args(["ls-tree", "-z", "--full-tree", commit.as_str(), "--"])
            .arg(path.as_str())
            .run()?;
        // `<mode> SP <type> SP <oid> TAB <path> NUL`
        let Some(entry) = out.split(|&b| b == 0).find(|e| !e.is_empty()) else {
            return Ok(None);
        };
        let entry = String::from_utf8_lossy(entry);
        let Some((meta, entry_path)) = entry.split_once('\t') else {
            return Err(YusraError::internal(format!(
                "output ls-tree tidak dikenali: {entry:?}"
            )));
        };
        let mut meta = meta.split(' ');
        let (Some(mode), Some(kind), Some(oid)) = (meta.next(), meta.next(), meta.next()) else {
            return Err(YusraError::internal(format!(
                "output ls-tree tidak dikenali: {entry:?}"
            )));
        };
        // Direktori, submodule, dan symlink bukan file biasa: symlink tidak
        // pernah diikuti, sama seperti saat indexing.
        if entry_path != path.as_str() || kind != "blob" || mode == "120000" {
            return Ok(None);
        }
        let oid = BlobId::parse(oid)
            .map_err(|e| YusraError::internal_with("oid ls-tree tidak valid", e))?;
        self.git()
            .args(["cat-file", "blob", oid.as_str()])
            .run()
            .map(Some)
    }

    fn changed_paths(&self, a: &CommitId, b: &CommitId) -> Result<Vec<RelPath>, YusraError> {
        self.verify_commit(a)?;
        self.verify_commit(b)?;
        let out = self
            .git()
            .args(["diff-tree", "-r", "-z", "--no-renames", "--name-only"])
            .args([a.as_str(), b.as_str()])
            .run()?;
        // Nama yang bukan UTF-8 atau ditolak `RelPath` dilewati, sama seperti
        // traversal saat indexing.
        let mut paths: Vec<RelPath> = out
            .split(|&b| b == 0)
            .filter_map(|p| std::str::from_utf8(p).ok())
            .filter(|p| !p.is_empty())
            .filter_map(|p| RelPath::parse(p).ok())
            .collect();
        paths.sort();
        Ok(paths)
    }

    fn ensure_worktree(&self, session: &SessionId, at: &CommitId) -> Result<PathBuf, YusraError> {
        self.verify_commit(at)?;
        let path = self.worktree_path(session);
        if self.owns_worktree(&path)? {
            // Kembalikan ke `at`: perubahan dan file untracked dibuang, file
            // yang di-ignore (cache build) dipertahankan.
            Git::new(&path)
                .args(["checkout", "--quiet", "--detach", "--force", at.as_str()])
                .run()?;
            Git::new(&path)
                .args(["clean", "--quiet", "-d", "--force", "--force"])
                .run()?;
            return Ok(path);
        }
        if path.exists() {
            return Err(YusraError::internal(format!(
                "{} sudah ada tetapi bukan worktree repo ini",
                path.display()
            )));
        }
        fs::create_dir_all(&self.worktrees_dir).map_err(|e| {
            YusraError::internal_with(format!("gagal membuat {}", self.worktrees_dir.display()), e)
        })?;
        // Pendaftaran worktree yang direktorinya sudah dihapus menghalangi
        // `worktree add` dengan path yang sama.
        self.git().args(["worktree", "prune"]).run()?;
        self.git()
            .args(["worktree", "add", "--quiet", "--detach"])
            .arg(&path)
            .arg(at.as_str())
            .run()?;
        Ok(path)
    }

    fn remove_worktree(&self, session: &SessionId) -> Result<(), YusraError> {
        let path = self.worktree_path(session);
        if self.owns_worktree(&path)? {
            // `--force` dua kali: hapus walau ada perubahan atau terkunci.
            self.git()
                .args(["worktree", "remove", "--force", "--force"])
                .arg(&path)
                .run()?;
        } else if path.exists() {
            return Err(YusraError::internal(format!(
                "{} bukan worktree repo ini; tidak dihapus",
                path.display()
            )));
        }
        self.git().args(["worktree", "prune"]).run()?;
        Ok(())
    }
}

/// Index sementara di direktori Git, dihapus saat keluar scope.
struct TempIndex(PathBuf);

impl TempIndex {
    fn new(path: PathBuf) -> Self {
        let index = Self(path);
        index.remove();
        index
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn remove(&self) {
        let _ = fs::remove_file(&self.0);
        let mut lock = self.0.clone().into_os_string();
        lock.push(".lock");
        let _ = fs::remove_file(lock);
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        self.remove();
    }
}

fn parse_oid<T>(
    stdout: &[u8],
    parse: impl Fn(&str) -> Result<T, yusra_core::ParseValueError>,
) -> Result<T, YusraError> {
    let text = String::from_utf8_lossy(stdout);
    parse(text.trim()).map_err(|e| YusraError::internal_with("oid dari git tidak valid", e))
}

fn canonical(path: &Path) -> Result<PathBuf, YusraError> {
    fs::canonicalize(path)
        .map_err(|e| YusraError::internal_with(format!("gagal membuka {}", path.display()), e))
}
