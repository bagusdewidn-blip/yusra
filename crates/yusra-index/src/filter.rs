//! Keputusan ignore per path untuk watcher, yang menerima path satu per satu.

use std::path::{Path, PathBuf};

use ignore::IncrementalIgnore;
use yusra_core::{RelPath, YusraError};

use crate::walk::{YUSRA_IGNORE_FILE, is_state_path, walk_builder};

/// Nama file aturan ignore; perubahannya membuat keputusan filter usang.
const IGNORE_RULE_FILES: [&str; 3] = [YUSRA_IGNORE_FILE, ".ignore", ".gitignore"];

/// Memutuskan apakah [`crate::walk`] akan melewati sebuah path, tanpa
/// menelusuri ulang. Dibangun dari konfigurasi traversal yang sama, jadi
/// keduanya selalu sepakat. Aturan per direktori dimuat saat pertama
/// dibutuhkan dan disimpan; panggil [`PathFilter::reload`] setelah file
/// aturan ignore berubah.
pub struct PathFilter {
    root: PathBuf,
    extra_ignore: Vec<String>,
    matcher: IncrementalIgnore,
}

impl PathFilter {
    pub fn new(root: &Path, extra_ignore: &[String]) -> Result<Self, YusraError> {
        Ok(Self {
            root: root.to_path_buf(),
            extra_ignore: extra_ignore.to_vec(),
            matcher: build_matcher(root, extra_ignore)?,
        })
    }

    /// Buang aturan yang sudah dimuat dan baca ulang dari disk.
    pub fn reload(&mut self) -> Result<(), YusraError> {
        self.matcher = build_matcher(&self.root, &self.extra_ignore)?;
        Ok(())
    }

    /// `true` jika path ini, atau salah satu direktori induknya, diabaikan.
    pub fn is_ignored(&mut self, path: &RelPath, is_dir: bool) -> bool {
        let in_state_dir = path
            .components()
            .enumerate()
            .any(|(i, part)| is_state_path(Some(part), i + 1));
        in_state_dir
            || self
                .matcher
                .matched(Path::new(path.as_str()), is_dir)
                .is_ignore()
    }
}

fn build_matcher(root: &Path, extra_ignore: &[String]) -> Result<IncrementalIgnore, YusraError> {
    walk_builder(root, extra_ignore)?
        .build_matchers()
        .into_iter()
        .next()
        .ok_or_else(|| YusraError::internal("matcher ignore tidak terbentuk"))
}

/// Path ini file aturan ignore atau `.git/info/exclude`.
pub(crate) fn is_ignore_rules_file(path: &RelPath) -> bool {
    let name = path.components().last().unwrap_or_default();
    IGNORE_RULE_FILES.contains(&name) || path.as_str() == ".git/info/exclude"
}
