//! Satu proyek yang dilayani: index di SQLite, cache tag untuk repo map, dan
//! operasi baca di balik tool MCP. Semua method sinkron; panggil dari kode
//! async lewat `spawn_blocking`.

use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::{Condvar, Mutex, PoisonError, RwLock, RwLockReadGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use yusra_core::{
    Hash32, ModelFamily, RelPath, SymbolKind, SymbolPath, TokenCount, TokenEstimator, YusraError,
};
use yusra_index::{Change, IndexedFile, LanguageAdapter, builtin_adapters, index_bytes, walk};
use yusra_store::{FileRecord, Store};
use yusra_tokens::{Estimator, FileTags, RepoMapRequest, repo_map};

use crate::{
    schema::{
        EstimateTokensRequest, EstimateTokensResponse, GetProjectMapRequest, GetProjectMapResponse,
        ReadFileRequest, ReadFileResponse, ReadSymbolRequest, ReadSymbolResponse,
        SearchSymbolsRequest, SearchSymbolsResponse, SymbolHit,
    },
    settings::{PathPolicy, Settings},
};

/// Direktori state di root proyek, dibuat oleh `yusrad init`.
pub const STATE_DIR: &str = ".yusra";
const DB_FILE: &str = "state.db";

const SEARCH_LIMIT_DEFAULT: u32 = 50;
const SEARCH_LIMIT_MAX: u32 = 200;
/// File yang ditulis ke database per transaksi saat indexing penuh.
const WRITE_BATCH: usize = 256;

/// Ringkasan satu putaran indexing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexStats {
    /// File terindeks setelah putaran ini.
    pub files: usize,
    /// File yang ditulis ulang ke database karena baru atau berubah.
    pub updated: usize,
    pub removed: usize,
    /// File yang dilewati karena `deny_read` atau gagal dibaca.
    pub skipped: usize,
}

/// Keadaan index awal. Tool dan watcher menunggu sampai bukan `Pending`.
#[derive(Debug)]
enum Readiness {
    Pending,
    Ready,
    Failed(String),
}

pub struct Project {
    root: PathBuf,
    settings: Settings,
    policy: PathPolicy,
    store: Store,
    adapters: Vec<Box<dyn LanguageAdapter>>,
    estimator: Estimator,
    tags: RwLock<BTreeMap<RelPath, FileTags>>,
    readiness: Mutex<Readiness>,
    ready_changed: Condvar,
}

impl std::fmt::Debug for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Project")
            .field("root", &self.root)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl Project {
    /// Buka proyek yang sudah di-`init` (direktori `.yusra/` harus ada).
    /// Index belum dimuat; panggil [`Project::full_index`]. Sampai itu
    /// selesai, [`Project::wait_ready`] memblok.
    pub fn open(root: &Path, settings: Settings) -> Result<Self, YusraError> {
        let root = fs::canonicalize(root).map_err(|e| {
            YusraError::internal_with(format!("root tidak bisa dibuka: {}", root.display()), e)
        })?;
        let state = root.join(STATE_DIR);
        if !state.is_dir() {
            return Err(YusraError::internal(format!(
                "{} belum ada; jalankan `yusrad init` di root proyek",
                state.display()
            )));
        }
        let store = Store::open(&state.join(DB_FILE))?;
        Ok(Self {
            policy: PathPolicy::new(&settings.deny_read)?,
            adapters: builtin_adapters()?,
            estimator: Estimator::new(),
            tags: RwLock::new(BTreeMap::new()),
            readiness: Mutex::new(Readiness::Pending),
            ready_changed: Condvar::new(),
            root,
            settings,
            store,
        })
    }

    fn set_readiness(&self, next: Readiness) {
        *self
            .readiness
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = next;
        self.ready_changed.notify_all();
    }

    /// Tandai index awal gagal: semua yang menunggu [`Project::wait_ready`]
    /// menerima error internal. Tidak berpengaruh jika index sudah siap.
    pub fn mark_index_failed(&self, reason: &str) {
        let mut state = self
            .readiness
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if matches!(*state, Readiness::Pending) {
            *state = Readiness::Failed(reason.to_owned());
            self.ready_changed.notify_all();
        }
    }

    /// Blok sampai [`Project::full_index`] pertama selesai.
    pub fn wait_ready(&self) -> Result<(), YusraError> {
        let state = self
            .readiness
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let state = self
            .ready_changed
            .wait_while(state, |s| matches!(s, Readiness::Pending))
            .unwrap_or_else(PoisonError::into_inner);
        match &*state {
            Readiness::Failed(reason) => {
                Err(YusraError::internal(format!("index awal gagal: {reason}")))
            }
            Readiness::Pending | Readiness::Ready => Ok(()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    fn tags(&self) -> RwLockReadGuard<'_, BTreeMap<RelPath, FileTags>> {
        self.tags.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Telusuri seluruh root, tulis file yang baru atau berubah ke database,
    /// hapus yang hilang, dan bangun ulang cache tag.
    pub fn full_index(&self) -> Result<IndexStats, YusraError> {
        let listing = walk(&self.root, &self.settings.extra_ignore)?;
        let known = self.store.file_hashes()?;
        let mut stats = IndexStats {
            skipped: listing.skipped.len(),
            ..IndexStats::default()
        };
        let mut seen = HashSet::new();
        let mut tags = BTreeMap::new();
        let mut pending = Vec::new();
        let now = now_ms();

        for entry in &listing.files {
            if self.policy.is_denied(&entry.path) {
                stats.skipped += 1;
                continue;
            }
            let (indexed, file_tags) = match self.index_one(&entry.path) {
                Ok(v) => v,
                Err(YusraError::NotFound { .. }) => continue,
                Err(e) => {
                    tracing::warn!(path = %entry.path, error = %e, "file dilewati");
                    stats.skipped += 1;
                    continue;
                }
            };
            seen.insert(entry.path.clone());
            tags.insert(entry.path.clone(), file_tags);
            if known.get(&entry.path) != Some(&indexed.hash) {
                pending.push(indexed);
                if pending.len() >= WRITE_BATCH {
                    stats.updated += self.write_indexed(&pending, now)?;
                    pending.clear();
                }
            }
        }
        stats.updated += self.write_indexed(&pending, now)?;

        let gone: Vec<RelPath> = known.into_keys().filter(|p| !seen.contains(p)).collect();
        stats.removed = self.store.remove_files(&gone)?;
        stats.files = tags.len();
        *self.tags.write().unwrap_or_else(PoisonError::into_inner) = tags;
        self.set_readiness(Readiness::Ready);
        Ok(stats)
    }

    /// Terapkan satu batch dari watcher.
    pub fn apply_changes(&self, changes: &[Change]) -> Result<IndexStats, YusraError> {
        if changes.iter().any(|c| matches!(c, Change::Rescan)) {
            return self.full_index();
        }
        let mut stats = IndexStats::default();
        let now = now_ms();
        for change in changes {
            match change {
                Change::Upsert(path) if !self.policy.is_denied(path) => {
                    match self.index_one(path) {
                        Ok((indexed, file_tags)) => {
                            stats.updated +=
                                self.write_indexed(std::slice::from_ref(&indexed), now)?;
                            self.tags
                                .write()
                                .unwrap_or_else(PoisonError::into_inner)
                                .insert(path.clone(), file_tags);
                        }
                        Err(YusraError::NotFound { .. }) => {
                            stats.removed += self.remove_tree(path)?
                        }
                        Err(e) => {
                            tracing::warn!(%path, error = %e, "file dilewati");
                            stats.skipped += 1;
                        }
                    }
                }
                // File yang kini cocok deny_read keluar dari index.
                Change::Upsert(path) | Change::Remove(path) => {
                    stats.removed += self.remove_tree(path)?;
                }
                Change::Rescan => {}
            }
        }
        stats.files = self.tags().len();
        Ok(stats)
    }

    /// Hapus `path` dan semua file di bawahnya dari index.
    fn remove_tree(&self, path: &RelPath) -> Result<usize, YusraError> {
        let prefix = format!("{path}/");
        let mut tags = self.tags.write().unwrap_or_else(PoisonError::into_inner);
        let doomed: Vec<RelPath> = tags
            .keys()
            .filter(|p| *p == path || p.as_str().starts_with(&prefix))
            .cloned()
            .collect();
        for p in &doomed {
            tags.remove(p);
        }
        drop(tags);
        self.store.remove_files(&doomed)
    }

    fn index_one(&self, path: &RelPath) -> Result<(IndexedFile, FileTags), YusraError> {
        let full = self.full_path(path);
        let meta = fs::symlink_metadata(&full).map_err(|e| io_error(e, path))?;
        if !meta.file_type().is_file() {
            return Err(not_found("file", path.as_str()));
        }
        let bytes = fs::read(&full).map_err(|e| io_error(e, path))?;
        let indexed = index_bytes(path, &bytes, &self.adapters, self.settings.max_file_bytes)?;
        let tags = indexed.parsed.as_ref().map_or(&[][..], |p| &p.tags[..]);
        let file_tags = FileTags::from_tags(path.clone(), tags, &bytes);
        Ok((indexed, file_tags))
    }

    fn write_indexed(&self, files: &[IndexedFile], now: i64) -> Result<usize, YusraError> {
        if files.is_empty() {
            return Ok(0);
        }
        let records: Vec<FileRecord<'_>> = files
            .iter()
            .map(|f| FileRecord {
                path: &f.path,
                hash: f.hash,
                language: f.language,
                indexed_at: now,
                symbols: f.parsed.as_ref().map_or(&[], |p| &p.symbols),
                // Resolusi import ke path belum ada; `read_file` memakai
                // rujukan tag untuk konteks dependensi.
                deps: &[],
            })
            .collect();
        self.store.replace_files(&records)?;
        Ok(files.len())
    }

    fn full_path(&self, path: &RelPath) -> PathBuf {
        let mut full = self.root.clone();
        full.extend(path.as_str().split('/'));
        full
    }

    /// Validasi path dari agen: `RelPath`, `deny_read`, lalu path kanonis
    /// (symlink diikuti) harus tetap di dalam root dan juga lolos
    /// `deny_read`. Mengembalikan path kanonis sebuah file biasa.
    fn resolve(&self, raw: &str) -> Result<(RelPath, PathBuf), YusraError> {
        let rel = RelPath::parse(raw)?;
        let denied = || YusraError::PathDenied {
            path: raw.to_owned(),
        };
        if self.policy.is_denied(&rel) {
            return Err(denied());
        }
        let canonical = fs::canonicalize(self.full_path(&rel)).map_err(|e| io_error(e, &rel))?;
        let Some(target) = canonical
            .strip_prefix(&self.root)
            .ok()
            .and_then(rel_from_components)
        else {
            return Err(denied());
        };
        if self.policy.is_denied(&target) {
            return Err(denied());
        }
        if !canonical.is_file() {
            return Err(not_found("file", raw));
        }
        Ok((rel, canonical))
    }

    fn reject_session(session_id: Option<&str>) -> Result<(), YusraError> {
        match session_id {
            Some(id) => Err(not_found("session", id)),
            None => Ok(()),
        }
    }

    pub fn search_symbols(
        &self,
        req: &SearchSymbolsRequest,
    ) -> Result<SearchSymbolsResponse, YusraError> {
        let limit = req
            .limit
            .unwrap_or(SEARCH_LIMIT_DEFAULT)
            .clamp(1, SEARCH_LIMIT_MAX);
        let kind = req.kind.map(SymbolKind::from);
        let symbols = self
            .store
            .search_symbols(&req.query, kind, limit)?
            .into_iter()
            .map(|s| SymbolHit {
                path: s.path.to_string(),
                symbol_path: s.symbol_path.0,
                kind: s.kind.as_str().to_owned(),
                signature: s.signature,
                hash: s.body_hash.to_hex(),
            })
            .collect();
        Ok(SearchSymbolsResponse { symbols })
    }

    pub fn project_map(
        &self,
        req: &GetProjectMapRequest,
    ) -> Result<GetProjectMapResponse, YusraError> {
        let focus = req
            .focus_paths
            .iter()
            .map(|p| RelPath::parse(p))
            .collect::<Result<Vec<_>, _>>()?;
        let tags = self.tags();
        let map = repo_map(
            tags.values(),
            &RepoMapRequest {
                focus_paths: &focus,
                mention_idents: &req.mention_idents,
                budget: req.max_tokens.unwrap_or(self.settings.repo_map_budget),
                family: ModelFamily::Generic,
            },
            &self.estimator,
        );
        Ok(GetProjectMapResponse {
            map: map.text,
            files_total: map.files_total,
            files_shown: map.files_shown,
            tokens: map.tokens.tokens,
        })
    }

    pub fn read_file(&self, req: &ReadFileRequest) -> Result<ReadFileResponse, YusraError> {
        let (rel, full) = self.resolve(&req.path)?;
        Self::reject_session(req.session_id.as_deref())?;
        let budget = req.max_tokens.unwrap_or(self.settings.default_read_budget);

        let (bytes, size) = read_prefix(&full, self.settings.max_file_bytes, &rel)?;
        let file_hash = if bytes.len() as u64 == size {
            hash_bytes(&bytes)
        } else {
            hash_file(&full, &rel)?
        };
        let text = String::from_utf8_lossy(&bytes);
        // Sisihkan sebagian anggaran untuk konteks dependensi; tanpa ini file
        // besar menghabiskan seluruh anggaran dan `deps_context` selalu kosong.
        let reserve = if req.with_deps {
            (budget / 4).min(self.settings.repo_map_budget)
        } else {
            0
        };
        let (content, content_tokens) = self.fit_text(&text, budget - reserve);
        let truncated = content.len() < text.len() || (bytes.len() as u64) < size;

        let mut tokens = content_tokens;
        let mut deps_context = None;
        let deps_budget = budget
            .saturating_sub(content_tokens.tokens)
            .min(self.settings.repo_map_budget);
        if req.with_deps && deps_budget > 0 {
            let tags = self.tags();
            let map = repo_map(
                tags.values(),
                &RepoMapRequest {
                    focus_paths: std::slice::from_ref(&rel),
                    mention_idents: &[],
                    budget: deps_budget,
                    family: ModelFamily::Generic,
                },
                &self.estimator,
            );
            if !map.text.is_empty() {
                tokens = tokens + map.tokens;
                deps_context = Some(map.text);
            }
        }

        Ok(ReadFileResponse {
            path: rel.to_string(),
            content: content.to_owned(),
            file_hash: file_hash.to_hex(),
            deps_context,
            truncated,
            tokens: tokens.tokens,
        })
    }

    /// Awalan terpanjang `text` yang muat dalam `budget`, dipotong di akhir
    /// baris; jika satu baris pun tidak muat, dipotong di batas karakter.
    fn fit_text<'t>(&self, text: &'t str, budget: u32) -> (&'t str, TokenCount) {
        let count = |s: &str| self.estimator.count(s, ModelFamily::Generic);
        let whole = count(text);
        if whole.tokens <= budget {
            return (text, whole);
        }
        let line_ends: Vec<usize> = text
            .match_indices('\n')
            .map(|(i, _)| i + 1)
            .chain(std::iter::once(text.len()))
            .collect();
        let mut cut = largest_fitting(line_ends.len(), |k| {
            count(&text[..line_ends[k - 1]]).tokens <= budget
        })
        .map_or(0, |k| line_ends[k - 1]);
        if cut == 0 {
            let first_line = line_ends.first().copied().unwrap_or(0);
            let chars: Vec<usize> = text[..first_line]
                .char_indices()
                .map(|(i, c)| i + c.len_utf8())
                .collect();
            cut = largest_fitting(chars.len(), |k| {
                count(&text[..chars[k - 1]]).tokens <= budget
            })
            .map_or(0, |k| chars[k - 1]);
        }
        let content = &text[..cut];
        (content, count(content))
    }

    pub fn read_symbol(&self, req: &ReadSymbolRequest) -> Result<ReadSymbolResponse, YusraError> {
        let (rel, full) = self.resolve(&req.path)?;
        Self::reject_session(req.session_id.as_deref())?;
        let missing = || not_found("symbol", &format!("{}#{}", req.path, req.symbol_path));
        let bytes = fs::read(&full).map_err(|e| io_error(e, &rel))?;
        let indexed = index_bytes(&rel, &bytes, &self.adapters, self.settings.max_file_bytes)?;
        let wanted = SymbolPath(req.symbol_path.clone());
        let symbol = indexed
            .parsed
            .as_ref()
            .and_then(|p| p.symbols.iter().find(|s| s.symbol_path == wanted))
            .ok_or_else(missing)?;
        let range = symbol.byte_range.clone();
        let source = bytes.get(range.clone()).ok_or_else(missing)?;
        let start_line = line_of(&bytes, range.start);
        let end_line = line_of(&bytes, range.end.saturating_sub(1).max(range.start));
        Ok(ReadSymbolResponse {
            source: String::from_utf8_lossy(source).into_owned(),
            hash: symbol.body_hash.to_hex(),
            start_line,
            end_line,
        })
    }

    pub fn estimate_tokens(
        &self,
        req: &EstimateTokensRequest,
    ) -> Result<EstimateTokensResponse, YusraError> {
        let family = ModelFamily::from(req.model_family);
        let mut total = req
            .text
            .as_deref()
            .map_or_else(TokenCount::default, |t| self.estimator.count(t, family));
        for raw in &req.paths {
            let (rel, full) = self.resolve(raw)?;
            let (bytes, size) = read_prefix(&full, self.settings.max_file_bytes, &rel)?;
            total = total
                + self
                    .estimator
                    .count(&String::from_utf8_lossy(&bytes), family);
            let rest = size.saturating_sub(bytes.len() as u64);
            if rest > 0 {
                // Sisa file besar diperkirakan dari ukurannya saja.
                let approx = u32::try_from(rest.div_ceil(3)).unwrap_or(u32::MAX);
                total = total + TokenCount::approx(approx);
            }
        }
        Ok(EstimateTokensResponse {
            tokens: total.tokens,
            approximate: total.approximate,
        })
    }
}

/// k terbesar di `1..=n` dengan `fits(k)`, dengan asumsi `fits` monoton.
fn largest_fitting(n: usize, fits: impl Fn(usize) -> bool) -> Option<usize> {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    (lo > 0).then_some(lo)
}

/// Nomor baris (mulai 1) dari byte `offset`.
fn line_of(bytes: &[u8], offset: usize) -> u32 {
    let end = offset.min(bytes.len());
    let newlines = bytes[..end].iter().filter(|&&b| b == b'\n').count();
    u32::try_from(newlines + 1).unwrap_or(u32::MAX)
}

fn rel_from_components(path: &Path) -> Option<RelPath> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?),
            _ => return None,
        }
    }
    RelPath::parse(&parts.join("/")).ok()
}

/// Baca paling banyak `cap` byte; kembalikan juga ukuran file sebenarnya.
fn read_prefix(full: &Path, cap: u64, rel: &RelPath) -> Result<(Vec<u8>, u64), YusraError> {
    let file = File::open(full).map_err(|e| io_error(e, rel))?;
    let size = file.metadata().map_err(|e| io_error(e, rel))?.len();
    let mut bytes = Vec::with_capacity(usize::try_from(size.min(cap)).unwrap_or(0));
    file.take(cap)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error(e, rel))?;
    Ok((bytes, size))
}

fn hash_bytes(bytes: &[u8]) -> Hash32 {
    yusra_index::hash_bytes(bytes)
}

fn hash_file(full: &Path, rel: &RelPath) -> Result<Hash32, YusraError> {
    let file = File::open(full).map_err(|e| io_error(e, rel))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file).map_err(|e| io_error(e, rel))?;
    Ok(Hash32(*hasher.finalize().as_bytes()))
}

fn not_found(kind: &'static str, id: &str) -> YusraError {
    YusraError::NotFound {
        kind,
        id: id.to_owned(),
    }
}

fn io_error(e: io::Error, path: &RelPath) -> YusraError {
    match e.kind() {
        io::ErrorKind::NotFound => not_found("file", path.as_str()),
        _ => YusraError::internal_with(format!("gagal membaca {path}"), e),
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
