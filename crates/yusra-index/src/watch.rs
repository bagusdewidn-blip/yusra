//! Pemantauan perubahan file berbasis event OS (inotify, FSEvents,
//! ReadDirectoryChangesW), tanpa polling.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
    time::Duration,
};

use notify_debouncer_full::{
    DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache, new_debouncer,
    notify::{
        EventKind, RecommendedWatcher, RecursiveMode,
        event::{AccessKind, AccessMode, ModifyKind},
    },
};
use yusra_core::{RelPath, YusraError};

use crate::{PathFilter, filter::is_ignore_rules_file, walk::rel_path};

/// Jeda debounce sesuai spesifikasi: penyimpanan beruntun dari editor
/// digabung menjadi satu perubahan.
pub const DEBOUNCE: Duration = Duration::from_millis(200);

/// Perubahan yang perlu diterapkan ke index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// File dibuat atau diubah dan tidak diabaikan; index ulang isinya.
    Upsert(RelPath),
    /// Path tidak ada lagi (atau kini symlink). Bisa berupa direktori: hapus
    /// path ini beserta semua yang ada di bawahnya. Path yang belum pernah
    /// di-index boleh diabaikan.
    Remove(RelPath),
    /// Event tidak lengkap, direktori baru muncul, atau aturan ignore
    /// berubah: telusuri ulang seluruh root dengan [`crate::walk`].
    Rescan,
}

/// Selama nilai ini hidup, perubahan dikirim ke callback `watch`.
/// Pemantauan berhenti saat nilai ini di-drop.
pub struct Watcher {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

/// Pantau `root` secara rekursif. `on_changes` dipanggil dari thread milik
/// debouncer dengan satu batch perubahan, urut menurut path, tanpa duplikat;
/// jika batch berisi [`Change::Rescan`], hanya itu yang dikirim. Aturan ignore
/// sama dengan [`crate::walk`].
pub fn watch(
    root: &Path,
    extra_ignore: &[String],
    mut on_changes: impl FnMut(Vec<Change>) + Send + 'static,
) -> Result<Watcher, YusraError> {
    // Path dari backend (terutama FSEvents) selalu kanonis; samakan rootnya.
    let root = fs::canonicalize(root).map_err(|e| {
        YusraError::internal_with(format!("root tidak bisa dibuka: {}", root.display()), e)
    })?;
    let mut filter = PathFilter::new(&root, extra_ignore)?;
    let handler_root = root.clone();
    let mut debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| {
        let changes = match result {
            Ok(events) => classify(&handler_root, &mut filter, &events),
            Err(_) => vec![Change::Rescan],
        };
        if changes == [Change::Rescan] {
            // Gagal memuat ulang tidak fatal: aturan lama tetap dipakai dan
            // penelusuran ulang tetap diminta.
            let _ = filter.reload();
        }
        if !changes.is_empty() {
            on_changes(changes);
        }
    })
    .map_err(|e| YusraError::internal_with("watcher tidak bisa dibuat", e))?;
    debouncer
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| YusraError::internal_with(format!("gagal memantau {}", root.display()), e))?;
    Ok(Watcher {
        _debouncer: debouncer,
    })
}

/// Ubah event mentah menjadi perubahan berdasarkan keadaan disk saat ini,
/// sehingga urutan event yang tidak pasti antar-OS tidak berpengaruh.
fn classify(root: &Path, filter: &mut PathFilter, events: &[DebouncedEvent]) -> Vec<Change> {
    let mut paths = BTreeSet::new();
    let mut appeared = BTreeSet::new();
    for event in events {
        if event.need_rescan() {
            return vec![Change::Rescan];
        }
        if is_read_only_access(&event.kind) {
            continue;
        }
        // Direktori yang dibuat atau dipindah masuk bisa sudah berisi file
        // yang tidak sempat menghasilkan event sendiri.
        let may_add_dir = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))
        );
        for p in &event.paths {
            paths.insert(p.clone());
            if may_add_dir {
                appeared.insert(p.clone());
            }
        }
    }

    let mut out = BTreeMap::new();
    for abs in paths {
        let Some(path) = rel_path(root, &abs) else {
            continue;
        };
        if is_ignore_rules_file(&path) {
            return vec![Change::Rescan];
        }
        match classify_path(filter, &abs, &path, appeared.contains(&abs)) {
            Some(Change::Rescan) => return vec![Change::Rescan],
            Some(change) => {
                out.insert(path, change);
            }
            None => {}
        }
    }
    out.into_values().collect()
}

/// Event akses tanpa penulisan (inotify `OPEN`, `CLOSE_NOWRITE`). Harus
/// dibuang: membaca file, termasuk saat filter memuat `.gitignore` atau
/// indexer membaca isi file, akan memicu event baru dan berputar tanpa akhir.
/// `Close(Write)` dipertahankan karena menandai penulisan yang selesai.
fn is_read_only_access(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Access(a) if *a != AccessKind::Close(AccessMode::Write))
}

fn classify_path(
    filter: &mut PathFilter,
    abs: &Path,
    path: &RelPath,
    appeared: bool,
) -> Option<Change> {
    match fs::symlink_metadata(abs) {
        Ok(meta) if meta.file_type().is_symlink() => {
            (!filter.is_ignored(path, false)).then(|| Change::Remove(path.clone()))
        }
        Ok(meta) if meta.is_file() => {
            (!filter.is_ignored(path, false)).then(|| Change::Upsert(path.clone()))
        }
        // Event lain pada direktori (mtime berubah karena isinya) tidak perlu
        // tindakan; isinya punya event sendiri.
        Ok(meta) if meta.is_dir() => {
            (appeared && !filter.is_ignored(path, true)).then_some(Change::Rescan)
        }
        Ok(_) => None,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Jenis path yang sudah hilang tidak diketahui. Lewati hanya jika
            // diabaikan baik sebagai file maupun direktori.
            let ignored = filter.is_ignored(path, false) && filter.is_ignored(path, true);
            (!ignored).then(|| Change::Remove(path.clone()))
        }
        // Tidak bisa dibaca (izin, dll.): tidak pasti, minta penelusuran ulang.
        Err(_) => Some(Change::Rescan),
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Instant};

    use notify_debouncer_full::notify::{
        Event,
        event::{CreateKind, DataChange, RemoveKind},
    };

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        filter: PathFilter,
    }

    impl Fixture {
        fn new(files: &[(&str, &str)]) -> Self {
            let dir = match tempfile::tempdir() {
                Ok(d) => d,
                Err(e) => panic!("{e}"),
            };
            let root = match fs::canonicalize(dir.path()) {
                Ok(r) => r,
                Err(e) => panic!("{e}"),
            };
            for (path, content) in files {
                let full = root.join(path);
                if let Some(parent) = full.parent()
                    && let Err(e) = fs::create_dir_all(parent)
                {
                    panic!("{e}");
                }
                if let Err(e) = fs::write(&full, content) {
                    panic!("{e}");
                }
            }
            let filter = match PathFilter::new(&root, &[]) {
                Ok(f) => f,
                Err(e) => panic!("{e}"),
            };
            Self {
                _dir: dir,
                root,
                filter,
            }
        }

        fn event(&self, kind: EventKind, path: &str) -> DebouncedEvent {
            DebouncedEvent::new(
                Event::new(kind).add_path(self.root.join(path)),
                Instant::now(),
            )
        }

        fn classify(&mut self, events: &[DebouncedEvent]) -> Vec<Change> {
            classify(&self.root, &mut self.filter, events)
        }
    }

    fn rel(raw: &str) -> RelPath {
        match RelPath::parse(raw) {
            Ok(p) => p,
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn reading_ignore_rules_is_not_a_change() {
        // Urutan inotify saat filter memuat `.gitignore` lalu file baru ditulis.
        let mut f = Fixture::new(&[(".gitignore", "*.log\n"), ("src/new.rs", "x")]);
        let events = [
            f.event(
                EventKind::Access(AccessKind::Open(AccessMode::Any)),
                ".gitignore",
            ),
            f.event(
                EventKind::Access(AccessKind::Close(AccessMode::Read)),
                ".gitignore",
            ),
            f.event(EventKind::Create(CreateKind::File), "src/new.rs"),
            f.event(
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                "src/new.rs",
            ),
            f.event(
                EventKind::Access(AccessKind::Close(AccessMode::Write)),
                "src/new.rs",
            ),
        ];
        assert_eq!(f.classify(&events), [Change::Upsert(rel("src/new.rs"))]);
    }

    #[test]
    fn writing_ignore_rules_rescans() {
        let mut f = Fixture::new(&[(".gitignore", "*.log\n")]);
        let events = [f.event(
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
            ".gitignore",
        )];
        assert_eq!(f.classify(&events), [Change::Rescan]);
    }

    #[test]
    fn ignored_and_state_paths_are_dropped() {
        let mut f = Fixture::new(&[
            (".gitignore", "*.log\n"),
            ("app.log", "x"),
            (".git/index", "x"),
            ("src/a.rs", "x"),
        ]);
        let modify = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        let events = [
            f.event(modify, "app.log"),
            f.event(modify, ".git/index"),
            f.event(modify, "src/a.rs"),
        ];
        assert_eq!(f.classify(&events), [Change::Upsert(rel("src/a.rs"))]);
    }

    #[test]
    fn missing_paths_are_removed() {
        let mut f = Fixture::new(&[]);
        let events = [f.event(EventKind::Remove(RemoveKind::File), "src/gone.rs")];
        assert_eq!(f.classify(&events), [Change::Remove(rel("src/gone.rs"))]);
    }

    #[test]
    fn new_directory_rescans_but_modified_directory_does_not() {
        let mut f = Fixture::new(&[("pkg/a.rs", "x")]);
        let touched = [f.event(EventKind::Modify(ModifyKind::Any), "pkg")];
        assert_eq!(f.classify(&touched), []);
        let created = [f.event(EventKind::Create(CreateKind::Folder), "pkg")];
        assert_eq!(f.classify(&created), [Change::Rescan]);
    }
}
