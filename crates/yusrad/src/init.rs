//! `yusrad init`: siapkan `.yusra/` di root proyek.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};
use yusra_mcp::STATE_DIR;

const EXCLUDE_LINE: &str = ".yusra/";

pub fn run(root: &Path) -> anyhow::Result<()> {
    let root = fs::canonicalize(root)
        .with_context(|| format!("root tidak bisa dibuka: {}", root.display()))?;
    if !root.is_dir() {
        bail!("{} bukan direktori", root.display());
    }
    #[cfg(target_os = "linux")]
    linux::reject_deploy_dir(&root)?;

    let state = root.join(STATE_DIR);
    fs::create_dir_all(&state).with_context(|| format!("gagal membuat {}", state.display()))?;
    println!("state: {}", state.display());

    match git_exclude_path(&root) {
        Some(exclude) => {
            if add_exclude(&exclude)? {
                println!("{EXCLUDE_LINE} ditambahkan ke {}", exclude.display());
            }
        }
        None => eprintln!(
            "peringatan: bukan repo Git (atau git tidak ada); {EXCLUDE_LINE} tidak ditambahkan \
             ke info/exclude"
        ),
    }

    #[cfg(target_os = "linux")]
    linux::warn_environment();
    Ok(())
}

/// Lokasi `info/exclude` repo, termasuk untuk worktree dan submodule.
fn git_exclude_path(root: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8(out.stdout).ok()?;
    let path = PathBuf::from(raw.trim());
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

/// Tambahkan baris `.yusra/` jika belum ada. `true` jika file berubah.
fn add_exclude(exclude: &Path) -> anyhow::Result<bool> {
    let current = match fs::read_to_string(exclude) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("gagal membaca {}", exclude.display())),
    };
    if current
        .lines()
        .any(|l| matches!(l.trim(), ".yusra" | ".yusra/" | "/.yusra" | "/.yusra/"))
    {
        return Ok(false);
    }
    if let Some(parent) = exclude.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("gagal membuat {}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(exclude)
        .with_context(|| format!("gagal membuka {}", exclude.display()))?;
    let sep = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    writeln!(file, "{sep}{EXCLUDE_LINE}")
        .with_context(|| format!("gagal menulis {}", exclude.display()))?;
    Ok(true)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{fs, path::Path};

    use anyhow::bail;

    const AUTO_DEPLOY_DIR: &str = "/etc/auto-deploy";
    const MIN_INOTIFY_WATCHES: u64 = 65_536;

    /// Tolak root yang sama dengan `APP_DIR` skrip auto-deploy: yusrad tidak
    /// boleh bekerja di direktori produksi. File dibaca sebagai teks, tidak
    /// pernah dieksekusi.
    pub fn reject_deploy_dir(root: &Path) -> anyhow::Result<()> {
        let Ok(entries) = fs::read_dir(AUTO_DEPLOY_DIR) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "conf") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for value in text
                .lines()
                .filter_map(|l| l.trim().strip_prefix("APP_DIR="))
            {
                let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
                let same = fs::canonicalize(value).is_ok_and(|dir| dir == root);
                if same {
                    bail!(
                        "{} adalah APP_DIR di {}; jalankan yusrad di clone dev terpisah",
                        root.display(),
                        path.display()
                    );
                }
            }
        }
        Ok(())
    }

    pub fn warn_environment() {
        match fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok())
        {
            Some(n) if n < MIN_INOTIFY_WATCHES => eprintln!(
                "peringatan: fs.inotify.max_user_watches = {n}; repo besar butuh >= \
                 {MIN_INOTIFY_WATCHES}"
            ),
            Some(_) => {}
            None => eprintln!("peringatan: fs.inotify.max_user_watches tidak bisa dibaca"),
        }
        let has_choom = std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("choom").is_file()));
        if !has_choom {
            eprintln!(
                "peringatan: choom tidak ditemukan; proses verifikasi tidak akan diprioritaskan \
                 untuk OOM killer"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_is_added_once() {
        let dir = match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => panic!("{e}"),
        };
        let exclude = dir.path().join("info").join("exclude");
        if let Err(e) = fs::create_dir_all(dir.path().join("info"))
            .and_then(|()| fs::write(&exclude, "# komentar\n*.log"))
        {
            panic!("{e}");
        }
        for expected in [true, false] {
            match add_exclude(&exclude) {
                Ok(changed) => assert_eq!(changed, expected),
                Err(e) => panic!("{e:#}"),
            }
        }
        match fs::read_to_string(&exclude) {
            Ok(t) => assert_eq!(t, "# komentar\n*.log\n.yusra/\n"),
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn exclude_file_created_when_missing() {
        let dir = match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => panic!("{e}"),
        };
        let exclude = dir.path().join("info").join("exclude");
        match add_exclude(&exclude) {
            Ok(changed) => assert!(changed),
            Err(e) => panic!("{e:#}"),
        }
        match fs::read_to_string(&exclude) {
            Ok(t) => assert_eq!(t, ".yusra/\n"),
            Err(e) => panic!("{e}"),
        }
    }
}
