use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use yusra_core::{CommitId, RelPath, SessionId, SnapshotId, Vcs, YusraError};
use yusra_git::{GitRepo, SNAPSHOT_REF_PREFIX};

const SESSION: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SNAP_A: &str = "01HZX5V8J9K0M1N2P3Q4R5S6T7";
const SNAP_B: &str = "01HZX5V8J9K0M1N2P3Q4R5S6T8";

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    worktrees: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = ok(tempfile::tempdir());
        let base = ok(fs::canonicalize(dir.path()));
        let root = base.join("repo");
        let worktrees = base.join("worktrees");
        ok(fs::create_dir_all(&root));
        let f = Self {
            _dir: dir,
            root,
            worktrees,
        };
        f.git(&["init", "--quiet", "--initial-branch=main"]);
        f
    }

    /// Jalankan git untuk menyiapkan atau memeriksa repo, terisolasi dari
    /// konfigurasi global mesin.
    fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let out = ok(Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=uji", "-c", "user.email=uji@example.com"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output());
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.root, args)
    }

    fn write(&self, path: &str, content: &str) {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            ok(fs::create_dir_all(parent));
        }
        ok(fs::write(full, content));
    }

    fn commit_all(&self, message: &str) -> CommitId {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
        ok(CommitId::parse(&self.git(&["rev-parse", "HEAD"])))
    }

    fn repo(&self) -> GitRepo {
        ok(GitRepo::open(&self.root, &self.worktrees))
    }

    /// Path file di tree commit, urut.
    fn tree_files(&self, commit: &CommitId) -> Vec<String> {
        let out = self.git(&["ls-tree", "-r", "--name-only", commit.as_str()]);
        out.lines().map(str::to_owned).collect()
    }
}

fn ok<T, E: std::fmt::Display>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

fn rel(raw: &str) -> RelPath {
    ok(RelPath::parse(raw))
}

fn snap(raw: &str) -> SnapshotId {
    ok(SnapshotId::parse(raw))
}

fn session() -> SessionId {
    ok(SessionId::parse(SESSION))
}

fn read(repo: &GitRepo, commit: &CommitId, path: &str) -> Option<String> {
    ok(repo.read_at(commit, &rel(path))).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn missing_commit() -> CommitId {
    ok(CommitId::parse(&"ab".repeat(20)))
}

#[test]
fn open_requires_repo_top_level() {
    let f = Fixture::new();
    f.write("sub/a.txt", "a\n");
    assert!(GitRepo::open(&f.root.join("sub"), &f.worktrees).is_err());

    let plain = ok(tempfile::tempdir());
    match GitRepo::open(plain.path(), &f.worktrees) {
        Err(YusraError::NotFound { kind, .. }) => assert_eq!(kind, "repo Git"),
        other => panic!("harus not_found, hasil {other:?}"),
    }
}

#[test]
fn unborn_head_and_parentless_snapshot() {
    let f = Fixture::new();
    f.write("a.txt", "a\n");
    let repo = f.repo();
    assert_eq!(ok(repo.head()), None);

    let commit = ok(repo.snapshot_worktree(&snap(SNAP_A)));
    assert_eq!(f.tree_files(&commit), ["a.txt"]);
    let parents = f.git(&["rev-list", "--parents", "-n", "1", commit.as_str()]);
    assert_eq!(
        parents,
        commit.as_str(),
        "snapshot tanpa commit tidak punya parent"
    );
    assert_eq!(ok(repo.head()), None, "HEAD tetap belum lahir");
}

#[test]
fn snapshot_records_worktree_without_touching_user_state() {
    let f = Fixture::new();
    f.write(".gitignore", "*.log\n");
    f.write("src/lib.rs", "pub fn a() {}\n");
    f.write("src/old.rs", "lama\n");
    let head = f.commit_all("awal");

    f.write("src/lib.rs", "pub fn a() { 1 }\n");
    f.write("src/new.rs", "baru\n");
    f.write("staged.rs", "di index saja\n");
    f.git(&["add", "staged.rs"]);
    ok(fs::remove_file(f.root.join("src/old.rs")));
    f.write("debug.log", "abaikan\n");
    f.write(".yusra/state.db", "state\n");

    let index_before = ok(fs::read(f.root.join(".git/index")));
    let status_before = f.git(&["status", "--porcelain"]);
    let refs_before = f.git(&["for-each-ref", "refs/heads", "refs/tags"]);

    let repo = f.repo();
    let commit = ok(repo.snapshot_worktree(&snap(SNAP_A)));

    assert_eq!(
        f.tree_files(&commit),
        [".gitignore", "src/lib.rs", "src/new.rs", "staged.rs"]
    );
    assert_eq!(
        read(&repo, &commit, "src/lib.rs").as_deref(),
        Some("pub fn a() { 1 }\n")
    );
    let parent = f.git(&["rev-parse", &format!("{commit}^")]);
    assert_eq!(parent, head.as_str());
    let snapshot_ref = format!("{SNAPSHOT_REF_PREFIX}{SNAP_A}");
    assert_eq!(f.git(&["rev-parse", &snapshot_ref]), commit.as_str());

    // Index, HEAD, branch, dan status pengguna tidak berubah.
    assert_eq!(ok(fs::read(f.root.join(".git/index"))), index_before);
    assert_eq!(ok(repo.head()), Some(head));
    assert_eq!(f.git(&["status", "--porcelain"]), status_before);
    assert_eq!(
        f.git(&["for-each-ref", "refs/heads", "refs/tags"]),
        refs_before
    );
    let leftovers: Vec<String> = ok(fs::read_dir(f.root.join(".git")))
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("yusra-index"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "index sementara tertinggal: {leftovers:?}"
    );

    // ID snapshot yang sama tidak boleh menimpa ref yang ada.
    assert!(repo.snapshot_worktree(&snap(SNAP_A)).is_err());
    assert_eq!(f.git(&["rev-parse", &snapshot_ref]), commit.as_str());
    assert!(repo.snapshot_worktree(&snap(SNAP_B)).is_ok());
}

#[test]
fn blobs_keep_exact_bytes() {
    let f = Fixture::new();
    f.git(&["config", "core.autocrlf", "true"]);
    let repo = f.repo();
    let bytes = b"baris\r\nkedua\r\n\x00\xff biner".to_vec();
    let id = ok(repo.write_blob(&bytes));
    assert_eq!(ok(repo.read_blob(&id)), bytes);
    assert_eq!(ok(repo.write_blob(&bytes)), id, "blob dialamati isinya");

    let empty = ok(repo.write_blob(b""));
    assert_eq!(ok(repo.read_blob(&empty)), b"");

    let missing = ok(yusra_core::BlobId::parse(&"cd".repeat(20)));
    assert!(matches!(
        repo.read_blob(&missing),
        Err(YusraError::NotFound { kind: "blob", .. })
    ));
    f.write("a.txt", "a\n");
    let commit = f.commit_all("awal");
    let not_blob = ok(yusra_core::BlobId::parse(commit.as_str()));
    assert!(matches!(
        repo.read_blob(&not_blob),
        Err(YusraError::NotFound { kind: "blob", .. })
    ));
}

#[test]
fn read_at_returns_only_regular_files() {
    let f = Fixture::new();
    f.write("src/main.rs", "fn main() {}\n");
    f.write("ab.txt", "ab\n");
    f.write("a b/c.txt", "spasi\n");
    #[cfg(unix)]
    ok(std::os::unix::fs::symlink(
        "ab.txt",
        f.root.join("link.txt"),
    ));
    let commit = f.commit_all("awal");
    let repo = f.repo();

    assert_eq!(
        read(&repo, &commit, "src/main.rs").as_deref(),
        Some("fn main() {}\n")
    );
    assert_eq!(
        read(&repo, &commit, "a b/c.txt").as_deref(),
        Some("spasi\n")
    );
    assert_eq!(read(&repo, &commit, "src/missing.rs"), None);
    assert_eq!(read(&repo, &commit, "src"), None, "direktori bukan file");
    // Pathspec literal: `a*.txt` bukan glob yang cocok dengan `ab.txt`.
    assert_eq!(read(&repo, &commit, "a*.txt"), None);
    #[cfg(unix)]
    assert_eq!(
        read(&repo, &commit, "link.txt"),
        None,
        "symlink tidak diikuti"
    );

    assert!(matches!(
        repo.read_at(&missing_commit(), &rel("src/main.rs")),
        Err(YusraError::NotFound { kind: "commit", .. })
    ));
}

#[test]
fn changed_paths_between_commits() {
    let f = Fixture::new();
    f.write("a.txt", "a\n");
    f.write("dir/b.txt", "b\n");
    f.write("dir/c.txt", "c\n");
    let first = f.commit_all("awal");
    f.write("a.txt", "a2\n");
    ok(fs::remove_file(f.root.join("dir/b.txt")));
    f.write("dir/sub/d.txt", "d\n");
    let second = f.commit_all("kedua");
    let repo = f.repo();

    let changed: Vec<String> = ok(repo.changed_paths(&first, &second))
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(changed, ["a.txt", "dir/b.txt", "dir/sub/d.txt"]);
    assert!(ok(repo.changed_paths(&second, &second)).is_empty());
    assert!(repo.changed_paths(&first, &missing_commit()).is_err());
}

#[test]
fn worktree_lifecycle() {
    let f = Fixture::new();
    f.write(".gitignore", "target/\n");
    f.write("a.txt", "satu\n");
    let first = f.commit_all("awal");
    f.write("a.txt", "dua\n");
    let second = f.commit_all("kedua");
    let head_before = f.git(&["symbolic-ref", "HEAD"]);
    let branches_before = f.git(&["for-each-ref", "refs/heads"]);
    let repo = f.repo();

    let path = ok(repo.ensure_worktree(&session(), &first));
    assert_eq!(path, f.worktrees.join(SESSION));
    assert_eq!(ok(fs::read_to_string(path.join("a.txt"))), "satu\n");

    // Isi worktree dikotori lalu dipindah ke commit lain.
    ok(fs::write(path.join("a.txt"), "kotor\n"));
    ok(fs::write(path.join("untracked.txt"), "x\n"));
    ok(fs::create_dir_all(path.join("target")));
    ok(fs::write(path.join("target/cache"), "build\n"));
    assert_eq!(ok(repo.ensure_worktree(&session(), &second)), path);
    assert_eq!(ok(fs::read_to_string(path.join("a.txt"))), "dua\n");
    assert!(!path.join("untracked.txt").exists());
    assert!(
        path.join("target/cache").exists(),
        "cache build dipertahankan"
    );

    // Worktree tidak memindahkan HEAD atau membuat branch pengguna.
    assert_eq!(f.git(&["symbolic-ref", "HEAD"]), head_before);
    assert_eq!(f.git(&["for-each-ref", "refs/heads"]), branches_before);
    assert_eq!(ok(fs::read_to_string(f.root.join("a.txt"))), "dua\n");

    ok(repo.remove_worktree(&session()));
    assert!(!path.exists());
    let listed = f.git(&["worktree", "list", "--porcelain"]);
    assert_eq!(listed.matches("worktree ").count(), 1, "{listed}");
    ok(repo.remove_worktree(&session()));

    // Direktori yang dihapus manual tidak menghalangi pembuatan ulang.
    let path = ok(repo.ensure_worktree(&session(), &first));
    ok(fs::remove_dir_all(&path));
    assert_eq!(ok(repo.ensure_worktree(&session(), &second)), path);
    ok(repo.remove_worktree(&session()));

    assert!(matches!(
        repo.ensure_worktree(&session(), &missing_commit()),
        Err(YusraError::NotFound { kind: "commit", .. })
    ));
}

#[test]
fn foreign_directory_is_never_touched() {
    let f = Fixture::new();
    f.write("a.txt", "a\n");
    let commit = f.commit_all("awal");
    let repo = f.repo();
    let path = repo.worktree_path(&session());
    ok(fs::create_dir_all(&path));
    ok(fs::write(path.join("milik-orang.txt"), "jangan dihapus\n"));

    assert!(repo.ensure_worktree(&session(), &commit).is_err());
    assert!(repo.remove_worktree(&session()).is_err());
    assert!(path.join("milik-orang.txt").exists());
}

#[test]
fn linked_worktree_root_uses_its_own_index() {
    let f = Fixture::new();
    f.write("a.txt", "a\n");
    let head = f.commit_all("awal");
    let linked = f.root.with_file_name("linked");
    f.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "fitur",
        linked.to_str().unwrap_or_default(),
    ]);
    ok(fs::write(linked.join("b.txt"), "b\n"));
    let main_index = ok(fs::read(f.root.join(".git/index")));

    let repo = ok(GitRepo::open(&linked, &f.worktrees));
    let commit = ok(repo.snapshot_worktree(&snap(SNAP_A)));
    assert_eq!(f.tree_files(&commit), ["a.txt", "b.txt"]);
    assert_eq!(ok(repo.head()), Some(head));
    assert_eq!(ok(fs::read(f.root.join(".git/index"))), main_index);

    // Worktree verifikasi dari linked worktree tetap dikenali sebagai milik repo.
    let path = ok(repo.ensure_worktree(&session(), &commit));
    assert_eq!(ok(repo.ensure_worktree(&session(), &commit)), path);
    ok(repo.remove_worktree(&session()));
    assert!(!path.exists());
}

#[test]
fn sha256_repository() {
    let f = Fixture::new();
    ok(fs::remove_dir_all(f.root.join(".git")));
    f.git(&["init", "--quiet", "--object-format=sha256"]);
    f.write("a.txt", "a\n");
    let head = f.commit_all("awal");
    assert_eq!(head.as_str().len(), 64);
    let repo = f.repo();
    let commit = ok(repo.snapshot_worktree(&snap(SNAP_A)));
    assert_eq!(commit.as_str().len(), 64);
    let blob = ok(repo.write_blob(b"x"));
    assert_eq!(blob.as_str().len(), 64);
    assert_eq!(read(&repo, &commit, "a.txt").as_deref(), Some("a\n"));
}
