//! Uji watcher terhadap event OS sungguhan. Batas waktu sengaja longgar
//! karena runner CI bisa lambat; uji tidak bergantung pada urutan batch.

mod common;

use std::{
    fs,
    path::Path,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use common::rel;
use yusra_index::{Change, watch};

const TIMEOUT: Duration = Duration::from_secs(15);

fn write(root: &Path, path: &str, content: &str) {
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

/// Kumpulkan perubahan sampai `done` terpenuhi; kembalikan semua yang diterima.
fn wait_until(rx: &Receiver<Vec<Change>>, done: impl Fn(&Change) -> bool) -> Vec<Change> {
    let deadline = Instant::now() + TIMEOUT;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(batch) => {
                let hit = batch.iter().any(&done);
                seen.extend(batch);
                if hit {
                    return seen;
                }
            }
            Err(_) => panic!("perubahan yang ditunggu tidak datang; diterima: {seen:?}"),
        }
    }
}

#[test]
fn reports_changes_and_respects_ignore_rules() {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    let root = dir.path();
    write(root, ".gitignore", "*.log\ntarget/\n");
    write(root, "src/lib.rs", "pub fn a() {}\n");

    let (tx, rx) = mpsc::channel();
    let _watcher = match watch(root, &[], move |batch| {
        let _ = tx.send(batch);
    }) {
        Ok(w) => w,
        Err(e) => panic!("{e}"),
    };
    // Beri waktu backend mendaftarkan pemantauan.
    thread::sleep(Duration::from_millis(300));

    // File baru.
    write(root, "src/new.rs", "pub fn b() {}\n");
    wait_until(&rx, |c| *c == Change::Upsert(rel("src/new.rs")));

    // File yang diabaikan tidak dilaporkan; sentinel memastikan batchnya sudah lewat.
    write(root, "app.log", "x\n");
    write(root, "target/out.rs", "x\n");
    write(root, "src/sentinel.rs", "x\n");
    let seen = wait_until(&rx, |c| *c == Change::Upsert(rel("src/sentinel.rs")));
    for c in &seen {
        if let Change::Upsert(p) | Change::Remove(p) = c {
            assert!(
                !p.as_str().ends_with(".log") && !p.as_str().starts_with("target"),
                "path diabaikan ikut dilaporkan: {c:?}"
            );
        }
    }

    // Penghapusan.
    if let Err(e) = fs::remove_file(root.join("src/new.rs")) {
        panic!("{e}");
    }
    wait_until(&rx, |c| *c == Change::Remove(rel("src/new.rs")));

    // Aturan ignore berubah: penelusuran ulang.
    write(root, ".gitignore", "*.log\n");
    wait_until(&rx, |c| *c == Change::Rescan);
}
