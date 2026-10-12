//! `yusrad up --stdio`: index proyek, pantau perubahan, layani MCP lewat
//! stdin/stdout. Log hanya ke stderr karena stdout milik protokol.

use std::{path::Path, sync::Arc, time::Instant};

use anyhow::Context;
use yusra_mcp::{Project, YusraServer, serve_stdio};

use crate::config::ProjectConfig;

/// `client_id` audit log untuk transport stdio (tanpa autentikasi).
const STDIO_CLIENT: &str = "stdio";

pub fn run(root: &Path, stdio: bool) -> anyhow::Result<()> {
    if !stdio {
        anyhow::bail!("transport HTTP belum tersedia; gunakan `yusrad up --stdio`");
    }
    let config = ProjectConfig::load(root)?;
    if !config.project.languages.is_empty() {
        tracing::warn!("[project] languages belum didukung; semua bahasa bawaan dipakai");
    }
    let settings = config.settings();
    let extra_ignore = settings.extra_ignore.clone();
    let project = Arc::new(Project::open(root, settings)?);

    let started = Instant::now();
    let stats = project.full_index()?;
    tracing::info!(
        files = stats.files,
        updated = stats.updated,
        removed = stats.removed,
        skipped = stats.skipped,
        elapsed_ms = started.elapsed().as_millis(),
        "index awal selesai"
    );

    let watched = Arc::clone(&project);
    let _watcher = yusra_index::watch(project.root(), &extra_ignore, move |changes| {
        let started = Instant::now();
        match watched.apply_changes(&changes) {
            Ok(stats) => tracing::debug!(
                changes = changes.len(),
                updated = stats.updated,
                removed = stats.removed,
                elapsed_ms = started.elapsed().as_millis(),
                "index diperbarui"
            ),
            Err(e) => tracing::error!(error = %e, "gagal memperbarui index"),
        }
    })?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("gagal membuat runtime tokio")?;
    runtime.block_on(serve_stdio(YusraServer::new(project, STDIO_CLIENT)))?;
    tracing::info!("klien menutup stdin, berhenti");
    Ok(())
}
