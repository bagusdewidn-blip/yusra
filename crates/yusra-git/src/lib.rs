//! Snapshot, worktree, dan akses objek Git lewat perintah `git`.

mod cmd;
mod repo;

pub use repo::{GitRepo, SNAPSHOT_REF_PREFIX};
