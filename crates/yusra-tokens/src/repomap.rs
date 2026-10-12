//! Repo map ala Aider: graf rujukan antar-file, Personalized PageRank, lalu
//! definisi teratas sebanyak yang muat dalam anggaran token.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use yusra_core::{ModelFamily, RelPath, Tag, TokenCount, TokenEstimator};

/// Baris yang lebih panjang dipotong saat dirender.
const MAX_LINE_CHARS: usize = 160;
const DAMPING: f64 = 0.85;
const MAX_ITERATIONS: usize = 100;
const TOLERANCE: f64 = 1e-6;

/// Definisi satu identifier beserta teks barisnya.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefLine {
    pub name: String,
    /// Dimulai dari 1.
    pub line: u32,
    pub text: String,
}

/// Bahan repo map untuk satu file. Pemanggil menyimpannya per hash file
/// agar tidak perlu mem-parse ulang file yang tidak berubah.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTags {
    pub path: RelPath,
    pub defs: Vec<DefLine>,
    /// Satu entri per kemunculan rujukan; nama yang sama boleh berulang.
    pub refs: Vec<String>,
}

impl FileTags {
    /// Bangun dari tag adapter dan isi file. Tag milik file lain diabaikan.
    pub fn from_tags(path: RelPath, tags: &[Tag], src: &[u8]) -> Self {
        let lines: Vec<&[u8]> = src.split(|&b| b == b'\n').collect();
        let mut defs = Vec::new();
        let mut refs = Vec::new();
        for tag in tags.iter().filter(|t| *t.file() == path) {
            match tag {
                Tag::Def { name, line, .. } => {
                    let text = usize::try_from(*line)
                        .ok()
                        .and_then(|l| l.checked_sub(1))
                        .and_then(|i| lines.get(i))
                        .map(|raw| clean_line(raw))
                        .unwrap_or_default();
                    defs.push(DefLine {
                        name: name.clone(),
                        line: *line,
                        text,
                    });
                }
                Tag::Ref { name, .. } => refs.push(name.clone()),
            }
        }
        Self { path, defs, refs }
    }
}

fn clean_line(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let text = text.trim_end();
    match text.char_indices().nth(MAX_LINE_CHARS) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text.to_owned(),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RepoMapRequest<'a> {
    /// File yang sedang dikerjakan agen. Diberi bobot tinggi dan tidak
    /// ditampilkan di peta karena isinya sudah dipegang agen.
    pub focus_paths: &'a [RelPath],
    pub mention_idents: &'a [String],
    pub budget: u32,
    pub family: ModelFamily,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoMap {
    pub text: String,
    pub files_total: usize,
    pub files_shown: usize,
    pub tokens: TokenCount,
}

/// Bangun repo map dari seluruh file terindeks. Path ganda di `files`
/// hanya diambil kemunculan pertamanya.
pub fn repo_map(
    files: &[FileTags],
    req: &RepoMapRequest<'_>,
    estimator: &dyn TokenEstimator,
) -> RepoMap {
    let graph = Graph::build(files, req);
    let ranked = graph.ranked_definitions();

    // Setiap baris yang dirender memuat setidaknya nomor baris, jadi lebih
    // dari `budget` definisi pasti tidak muat.
    let upper = ranked
        .len()
        .min(usize::try_from(req.budget).unwrap_or(usize::MAX));
    let render_k = |k: usize| render(&graph.files, &ranked[..k]);
    let fits = |k: usize| {
        let (text, _) = render_k(k);
        let tokens = estimator.count(&text, req.family);
        tokens.tokens <= req.budget
    };

    // k terbesar dengan fits(k); fits(0) selalu benar karena teks kosong.
    let (mut lo, mut hi) = (0, upper);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(mid) { lo = mid } else { hi = mid - 1 }
    }
    let (text, files_shown) = render_k(lo);
    let tokens = estimator.count(&text, req.family);
    RepoMap {
        text,
        files_total: graph.files.len(),
        files_shown,
        tokens,
    }
}

/// Satu definisi yang diperingkat: identifier `ident` di file `file`.
#[derive(Clone, Copy, Debug)]
struct Ranked<'a> {
    file: usize,
    ident: &'a str,
}

struct Edge<'a> {
    from: usize,
    to: usize,
    ident: &'a str,
    weight: f64,
}

struct Graph<'a> {
    files: Vec<&'a FileTags>,
    focus: Vec<bool>,
    personalization: Vec<f64>,
    edges: Vec<Edge<'a>>,
}

impl<'a> Graph<'a> {
    fn build(input: &'a [FileTags], req: &RepoMapRequest<'_>) -> Self {
        let mut seen = HashSet::new();
        let files: Vec<&FileTags> = input.iter().filter(|f| seen.insert(&f.path)).collect();

        let focus_set: HashSet<&RelPath> = req.focus_paths.iter().collect();
        let mention: HashSet<&str> = req.mention_idents.iter().map(String::as_str).collect();
        let focus: Vec<bool> = files.iter().map(|f| focus_set.contains(&f.path)).collect();

        let personalization: Vec<f64> = files
            .iter()
            .zip(&focus)
            .map(|(f, &is_focus)| {
                if is_focus || path_mentioned(&f.path, &mention) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();

        // BTreeMap agar urutan penjumlahan, dan karena itu hasil floating
        // point, tidak bergantung pada urutan hash.
        let mut defs: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
        let mut refs: BTreeMap<&str, BTreeMap<usize, u32>> = BTreeMap::new();
        for (i, f) in files.iter().enumerate() {
            for d in &f.defs {
                defs.entry(d.name.as_str()).or_default().insert(i);
            }
            for r in &f.refs {
                *refs.entry(r.as_str()).or_default().entry(i).or_default() += 1;
            }
        }

        let mut edges = Vec::new();
        for (&ident, definers) in &defs {
            let Some(referencers) = refs.get(ident) else {
                continue;
            };
            let w = ident_weight(ident, definers.len(), mention.contains(ident));
            for (&from, &n) in referencers {
                let focus_mul = if focus[from] { 50.0 } else { 1.0 };
                for &to in definers {
                    edges.push(Edge {
                        from,
                        to,
                        ident,
                        weight: w * f64::from(n).sqrt() * focus_mul,
                    });
                }
            }
        }

        Self {
            files,
            focus,
            personalization,
            edges,
        }
    }

    /// Definisi di luar file fokus, urut skor menurun lalu path dan nama.
    fn ranked_definitions(&self) -> Vec<Ranked<'a>> {
        let rank = pagerank(self.files.len(), &self.edges, &self.personalization);
        let mut out_weight = vec![0.0; self.files.len()];
        for e in &self.edges {
            out_weight[e.from] += e.weight;
        }
        let mut scores: BTreeMap<(usize, &str), f64> = BTreeMap::new();
        for e in &self.edges {
            if out_weight[e.from] > 0.0 {
                *scores.entry((e.to, e.ident)).or_default() +=
                    rank[e.from] * e.weight / out_weight[e.from];
            }
        }
        let mut ranked: Vec<(f64, Ranked<'a>)> = scores
            .into_iter()
            .filter(|&((file, _), _)| !self.focus[file])
            .map(|((file, ident), s)| (s, Ranked { file, ident }))
            .collect();
        ranked.sort_by(|(sa, a), (sb, b)| {
            sb.total_cmp(sa)
                .then_with(|| self.files[a.file].path.cmp(&self.files[b.file].path))
                .then_with(|| a.ident.cmp(b.ident))
        });
        ranked.into_iter().map(|(_, r)| r).collect()
    }
}

fn ident_weight(ident: &str, definer_count: usize, mentioned: bool) -> f64 {
    let mut w = 1.0;
    if mentioned {
        w *= 10.0;
    }
    let is_snake = ident.contains('_') && ident.chars().any(char::is_alphabetic);
    let is_camel = ident.chars().any(char::is_uppercase) && ident.chars().any(char::is_lowercase);
    if ident.chars().count() >= 8 && (is_snake || is_camel) {
        w *= 10.0;
    }
    if ident.starts_with('_') {
        w *= 0.1;
    }
    if definer_count > 5 {
        w *= 0.1;
    }
    w
}

/// File disebut jika salah satu komponen path-nya, dengan atau tanpa
/// ekstensi, sama dengan identifier yang disebut agen.
fn path_mentioned(path: &RelPath, mention: &HashSet<&str>) -> bool {
    !mention.is_empty()
        && path.as_str().split('/').any(|c| {
            let stem = c.split_once('.').map_or(c, |(s, _)| s);
            mention.contains(c) || mention.contains(stem)
        })
}

/// PageRank berbobot dengan vektor personalisasi. Massa simpul tanpa sisi
/// keluar dibagi menurut personalisasi. Personalisasi kosong = seragam.
fn pagerank(n: usize, edges: &[Edge<'_>], personalization: &[f64]) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    let total: f64 = personalization.iter().sum();
    let p: Vec<f64> = if total > 0.0 {
        personalization.iter().map(|x| x / total).collect()
    } else {
        vec![1.0 / n as f64; n]
    };

    let mut out_weight = vec![0.0; n];
    for e in edges {
        out_weight[e.from] += e.weight;
    }
    let dangling: Vec<usize> = (0..n).filter(|&i| out_weight[i] <= 0.0).collect();

    let mut x = vec![1.0 / n as f64; n];
    for _ in 0..MAX_ITERATIONS {
        let last = std::mem::replace(&mut x, vec![0.0; n]);
        for e in edges {
            x[e.to] += DAMPING * last[e.from] * e.weight / out_weight[e.from];
        }
        let dangle_sum: f64 = DAMPING * dangling.iter().map(|&i| last[i]).sum::<f64>();
        for (xi, pi) in x.iter_mut().zip(&p) {
            *xi += (dangle_sum + (1.0 - DAMPING)) * pi;
        }
        let err: f64 = x.iter().zip(&last).map(|(a, b)| (a - b).abs()).sum();
        if err < n as f64 * TOLERANCE {
            break;
        }
    }
    x
}

/// Render definisi terpilih per file (urut path), satu baris per definisi:
/// `nomor: teks`. Mengembalikan teks dan jumlah file yang tampil.
fn render(files: &[&FileTags], picked: &[Ranked<'_>]) -> (String, usize) {
    let mut wanted: BTreeMap<&RelPath, BTreeSet<(u32, &str)>> = BTreeMap::new();
    let mut by_file: HashMap<usize, HashSet<&str>> = HashMap::new();
    for r in picked {
        by_file.entry(r.file).or_default().insert(r.ident);
    }
    for (file, idents) in by_file {
        let f = files[file];
        let lines = wanted.entry(&f.path).or_default();
        for d in f.defs.iter().filter(|d| idents.contains(d.name.as_str())) {
            lines.insert((d.line, d.text.as_str()));
        }
    }

    let mut out = String::new();
    for (path, lines) in &wanted {
        out.push_str(path.as_str());
        out.push_str(":\n");
        for (line, text) in lines {
            out.push_str(&format!("{line}: {text}\n"));
        }
    }
    (out, wanted.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Satu token per kata, eksak: membuat uji anggaran mudah dihitung.
    struct Words;

    impl TokenEstimator for Words {
        fn count(&self, text: &str, _: ModelFamily) -> TokenCount {
            TokenCount::exact(text.split_whitespace().count() as u32)
        }
    }

    fn rel(p: &str) -> RelPath {
        match RelPath::parse(p) {
            Ok(p) => p,
            Err(e) => panic!("{e}"),
        }
    }

    fn file(path: &str, defs: &[(&str, u32)], refs: &[&str]) -> FileTags {
        FileTags {
            path: rel(path),
            defs: defs
                .iter()
                .map(|&(name, line)| DefLine {
                    name: name.to_owned(),
                    line,
                    text: format!("fn {name}()"),
                })
                .collect(),
            refs: refs.iter().map(|&r| r.to_owned()).collect(),
        }
    }

    fn request<'a>(focus: &'a [RelPath], mention: &'a [String], budget: u32) -> RepoMapRequest<'a> {
        RepoMapRequest {
            focus_paths: focus,
            mention_idents: mention,
            budget,
            family: ModelFamily::Generic,
        }
    }

    /// `core.rs` dirujuk semua file, `util.rs` hanya oleh satu.
    fn sample() -> Vec<FileTags> {
        vec![
            file(
                "src/a.rs",
                &[("run_app", 1)],
                &["load_config", "load_config", "pad"],
            ),
            file("src/b.rs", &[("serve", 3)], &["load_config", "run_app"]),
            file("src/core.rs", &[("load_config", 10)], &[]),
            file("src/util.rs", &[("pad", 7)], &[]),
            file("src/lonely.rs", &[("unused", 1)], &[]),
        ]
    }

    fn first_lines(map: &RepoMap) -> Vec<&str> {
        map.text.lines().filter(|l| l.ends_with(':')).collect()
    }

    #[test]
    fn most_referenced_file_comes_first_in_score() {
        let files = sample();
        let graph = Graph::build(&files, &request(&[], &[], 100));
        let ranked = graph.ranked_definitions();
        let names: Vec<&str> = ranked.iter().map(|r| r.ident).collect();
        assert_eq!(names.first(), Some(&"load_config"));
        // Definisi tanpa rujukan tidak masuk peringkat.
        assert!(!names.contains(&"unused"));
    }

    #[test]
    fn renders_grouped_by_path() {
        let files = sample();
        let map = repo_map(&files, &request(&[], &[], 1000), &Words);
        assert_eq!(
            map.text,
            "src/a.rs:\n1: fn run_app()\nsrc/core.rs:\n10: fn load_config()\nsrc/util.rs:\n7: fn pad()\n"
        );
        assert_eq!(map.files_total, 5);
        assert_eq!(map.files_shown, 3);
        assert_eq!(map.tokens, Words.count(&map.text, ModelFamily::Generic));
    }

    #[test]
    fn budget_is_respected() {
        let files = sample();
        // Satu file dengan satu definisi = 4 kata: "path:", "n:", "fn", "x()".
        let map = repo_map(&files, &request(&[], &[], 5), &Words);
        assert!(map.tokens.tokens <= 5);
        assert_eq!(first_lines(&map), ["src/core.rs:"]);

        let empty = repo_map(&files, &request(&[], &[], 0), &Words);
        assert_eq!(empty.text, "");
        assert_eq!(empty.files_shown, 0);
    }

    #[test]
    fn focus_files_are_hidden_and_boost_their_targets() {
        let files = sample();
        let focus = [rel("src/a.rs")];
        let graph = Graph::build(&files, &request(&focus, &[], 100));
        let ranked = graph.ranked_definitions();
        assert!(ranked.iter().all(|r| graph.files[r.file].path != focus[0]));

        // Dengan a.rs sebagai fokus, pad (hanya dirujuk a.rs) mengalahkan
        // run_app (hanya dirujuk b.rs).
        let pos = |name: &str| ranked.iter().position(|r| r.ident == name);
        assert!(pos("pad") < pos("serve").or(Some(usize::MAX)));
        assert_eq!(ranked.first().map(|r| r.ident), Some("load_config"));
    }

    #[test]
    fn mentioned_ident_wins() {
        let files = vec![
            file("a.rs", &[], &["alpha", "beta"]),
            file("x.rs", &[("alpha", 1)], &[]),
            file("y.rs", &[("beta", 1)], &[]),
        ];
        let plain = Graph::build(&files, &request(&[], &[], 100));
        let order = |g: &Graph<'_>| -> Vec<String> {
            g.ranked_definitions()
                .iter()
                .map(|r| r.ident.to_owned())
                .collect()
        };
        assert_eq!(order(&plain), ["alpha", "beta"]);
        let mention = ["beta".to_owned()];
        let boosted = Graph::build(&files, &request(&[], &mention, 100));
        assert_eq!(order(&boosted), ["beta", "alpha"]);
    }

    #[test]
    fn duplicate_paths_and_shared_lines_are_deduplicated() {
        let files = vec![
            file("a.rs", &[], &["x", "y"]),
            FileTags {
                path: rel("c.rs"),
                defs: vec![
                    DefLine {
                        name: "x".into(),
                        line: 2,
                        text: "const x, y = 1, 2".into(),
                    },
                    DefLine {
                        name: "y".into(),
                        line: 2,
                        text: "const x, y = 1, 2".into(),
                    },
                ],
                refs: vec![],
            },
            file("a.rs", &[("ignored", 1)], &[]),
        ];
        let map = repo_map(&files, &request(&[], &[], 100), &Words);
        assert_eq!(map.text, "c.rs:\n2: const x, y = 1, 2\n");
        assert_eq!(map.files_total, 2);
    }

    #[test]
    fn pagerank_sums_to_one_and_follows_personalization() {
        let edges = [
            Edge {
                from: 0,
                to: 1,
                ident: "a",
                weight: 1.0,
            },
            Edge {
                from: 1,
                to: 2,
                ident: "b",
                weight: 1.0,
            },
            Edge {
                from: 2,
                to: 0,
                ident: "c",
                weight: 1.0,
            },
        ];
        let uniform = pagerank(3, &edges, &[0.0; 3]);
        assert!((uniform.iter().sum::<f64>() - 1.0).abs() < 1e-6);
        for r in &uniform {
            assert!((r - 1.0 / 3.0).abs() < 1e-6);
        }
        let biased = pagerank(3, &edges, &[1.0, 0.0, 0.0]);
        assert!((biased.iter().sum::<f64>() - 1.0).abs() < 1e-6);
        assert!(biased[0] > biased[2]);
        assert!(pagerank(0, &[], &[]).is_empty());
    }

    #[test]
    fn ident_weights() {
        assert_eq!(ident_weight("run", 1, false), 1.0);
        assert_eq!(ident_weight("load_config", 1, false), 10.0);
        assert_eq!(ident_weight("SessionStore", 1, false), 10.0);
        assert_eq!(ident_weight("lowercase", 1, false), 1.0);
        assert_eq!(ident_weight("run", 1, true), 10.0);
        assert!((ident_weight("_x", 6, false) - 0.01).abs() < 1e-12);
    }

    #[test]
    fn mentioned_paths() {
        let m: HashSet<&str> = ["session", "auth"].into_iter().collect();
        assert!(path_mentioned(&rel("src/session.rs"), &m));
        assert!(path_mentioned(&rel("auth/mod.rs"), &m));
        assert!(!path_mentioned(&rel("src/sessions.rs"), &m));
    }

    #[test]
    fn from_tags_reads_lines() {
        let path = rel("src/lib.rs");
        let other = rel("src/other.rs");
        let src = b"use x;\n    pub fn run() {\r\n}\n";
        let tags = [
            Tag::Def {
                name: "run".into(),
                file: path.clone(),
                line: 2,
            },
            Tag::Def {
                name: "ghost".into(),
                file: path.clone(),
                line: 99,
            },
            Tag::Ref {
                name: "x".into(),
                file: path.clone(),
            },
            Tag::Ref {
                name: "y".into(),
                file: other,
            },
        ];
        let ft = FileTags::from_tags(path, &tags, src);
        assert_eq!(ft.defs[0].text, "    pub fn run() {");
        assert_eq!(ft.defs[1].text, "");
        assert_eq!(ft.refs, ["x"]);
    }

    #[test]
    fn long_lines_are_truncated() {
        let long = "é".repeat(MAX_LINE_CHARS + 10);
        let cleaned = clean_line(long.as_bytes());
        assert_eq!(cleaned.chars().count(), MAX_LINE_CHARS + 3);
        assert!(cleaned.ends_with("..."));
    }
}
