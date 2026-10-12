use ignore::gitignore::{Gitignore, GitignoreBuilder};
use yusra_core::{RelPath, YusraError};

/// Bagian konfigurasi proyek yang dipakai server MCP. `yusrad` mengisinya
/// dari `yusra.toml`; nilai default sama dengan default di spesifikasi.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// File yang lebih besar hanya di-hash, tidak di-parse, dan dibaca
    /// sebagian oleh `read_file`.
    pub max_file_bytes: u64,
    pub extra_ignore: Vec<String>,
    /// Pola bergaya gitignore. File yang cocok tidak di-index dan tidak bisa
    /// dibaca agen.
    pub deny_read: Vec<String>,
    pub default_read_budget: u32,
    pub repo_map_budget: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_file_bytes: 1024 * 1024,
            extra_ignore: Vec::new(),
            deny_read: [
                ".env", ".env.*", "*.pem", "*.key", "id_rsa*", "*.p12", "*.pfx",
            ]
            .map(str::to_owned)
            .to_vec(),
            default_read_budget: 8000,
            repo_map_budget: 2048,
        }
    }
}

/// Direktori yang tidak pernah bisa dibaca agen, di kedalaman mana pun.
const INTERNAL_DIRS: [&str; 2] = [".git", ".yusra"];

/// Keputusan baca per path: `deny_read` ditambah direktori internal.
#[derive(Debug)]
pub struct PathPolicy {
    deny: Gitignore,
}

impl PathPolicy {
    pub fn new(deny_read: &[String]) -> Result<Self, YusraError> {
        // Root "." membuat matcher tidak mencoba membuang prefiks dari path
        // relatif yang diberikan.
        let mut builder = GitignoreBuilder::new(".");
        for pattern in deny_read {
            builder.add_line(None, pattern).map_err(|e| {
                YusraError::internal_with(format!("pola deny_read tidak valid: {pattern:?}"), e)
            })?;
        }
        let deny = builder
            .build()
            .map_err(|e| YusraError::internal_with("pola deny_read tidak valid", e))?;
        Ok(Self { deny })
    }

    pub fn is_denied(&self, path: &RelPath) -> bool {
        path.as_str().split('/').any(|c| INTERNAL_DIRS.contains(&c))
            || self
                .deny
                .matched_path_or_any_parents(path.as_str(), false)
                .is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(p: &str) -> RelPath {
        match RelPath::parse(p) {
            Ok(p) => p,
            Err(e) => panic!("{e}"),
        }
    }

    fn policy(patterns: &[&str]) -> PathPolicy {
        let patterns: Vec<String> = patterns.iter().map(|p| (*p).to_owned()).collect();
        match PathPolicy::new(&patterns) {
            Ok(p) => p,
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn default_patterns() {
        let p = policy(
            &Settings::default()
                .deny_read
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
        for denied in [
            ".env",
            "config/.env",
            ".env.local",
            "certs/server.pem",
            "home/id_rsa.pub",
            ".git/config",
            "vendor/lib/.git/HEAD",
            ".yusra/state.db",
        ] {
            assert!(p.is_denied(&rel(denied)), "{denied} harus ditolak");
        }
        for allowed in [
            "src/env.rs",
            "environment.ts",
            "src/keys.rs",
            ".github/ci.yml",
        ] {
            assert!(!p.is_denied(&rel(allowed)), "{allowed} harus boleh");
        }
    }

    #[test]
    fn directories_and_negation() {
        let p = policy(&["secrets/", "*.json", "!package.json"]);
        assert!(p.is_denied(&rel("secrets/a.txt")));
        assert!(p.is_denied(&rel("app/secrets/deep/b.txt")));
        assert!(p.is_denied(&rel("data/x.json")));
        assert!(!p.is_denied(&rel("package.json")));
    }

    #[test]
    fn anchored_pattern() {
        let p = policy(&["/local.toml"]);
        assert!(p.is_denied(&rel("local.toml")));
        assert!(!p.is_denied(&rel("sub/local.toml")));
    }
}
