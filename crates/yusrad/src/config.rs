//! `yusra.toml` di root proyek. Semua bagian opsional; field yang tidak
//! dikenal ditolak agar salah ketik tidak diam-diam diabaikan.

use std::{fs, io, path::Path};

use anyhow::Context;
use serde::Deserialize;
use yusra_mcp::Settings;

pub const CONFIG_FILE: &str = "yusra.toml";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct ProjectConfig {
    pub project: ProjectSection,
    pub index: IndexSection,
    pub security: SecuritySection,
    pub session: SessionSection,
    pub tokens: TokensSection,
    pub verify: Vec<VerifyBlock>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct ProjectSection {
    pub name: Option<String>,
    /// Kosong = deteksi dari ekstensi.
    pub languages: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct IndexSection {
    pub max_file_bytes: u64,
    pub extra_ignore: Vec<String>,
}

impl Default for IndexSection {
    fn default() -> Self {
        let s = Settings::default();
        Self {
            max_file_bytes: s.max_file_bytes,
            extra_ignore: s.extra_ignore,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct SecuritySection {
    pub deny_read: Vec<String>,
    pub flag_in_review: Vec<String>,
}

impl Default for SecuritySection {
    fn default() -> Self {
        Self {
            deny_read: Settings::default().deny_read,
            flag_in_review: [
                "build.rs",
                "package.json",
                "Makefile",
                ".github/**",
                "**/*.sh",
            ]
            .map(str::to_owned)
            .to_vec(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct SessionSection {
    pub lock_ttl_secs: u64,
    pub stall_after_secs: u64,
}

impl Default for SessionSection {
    fn default() -> Self {
        Self {
            lock_ttl_secs: 120,
            stall_after_secs: 600,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct TokensSection {
    pub default_read_budget: u32,
    pub repo_map_budget: u32,
}

impl Default for TokensSection {
    fn default() -> Self {
        let s = Settings::default();
        Self {
            default_read_budget: s.default_read_budget,
            repo_map_budget: s.repo_map_budget,
        }
    }
}

/// Perintah verifikasi, dijalankan sebagai argv tanpa shell.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VerifyBlock {
    pub name: String,
    pub run: Vec<String>,
    #[serde(default = "default_verify_timeout")]
    pub timeout_secs: u64,
}

fn default_verify_timeout() -> u64 {
    600
}

impl ProjectConfig {
    /// Muat `yusra.toml` dari `root`; default jika file tidak ada.
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join(CONFIG_FILE);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("gagal membaca {}", path.display())),
        };
        Self::parse(&text).with_context(|| format!("{} tidak valid", path.display()))
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let config: Self = toml::from_str(text)?;
        for v in &config.verify {
            if v.run.is_empty() {
                anyhow::bail!("[[verify]] {:?}: `run` tidak boleh kosong", v.name);
            }
        }
        Ok(config)
    }

    pub fn settings(&self) -> Settings {
        Settings {
            max_file_bytes: self.index.max_file_bytes,
            extra_ignore: self.index.extra_ignore.clone(),
            deny_read: self.security.deny_read.clone(),
            default_read_budget: self.tokens.default_read_budget,
            repo_map_budget: self.tokens.repo_map_budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> ProjectConfig {
        match ProjectConfig::parse(text) {
            Ok(c) => c,
            Err(e) => panic!("{e:#}"),
        }
    }

    #[test]
    fn empty_is_default() {
        assert_eq!(parse(""), ProjectConfig::default());
        assert_eq!(parse("").settings(), Settings::default());
    }

    #[test]
    fn full_example() {
        let c = parse(
            r#"
[project]
name = "my-app"
languages = []

[index]
max_file_bytes = 2048
extra_ignore = ["dist/", "*.min.js"]

[security]
deny_read = [".env"]
flag_in_review = ["build.rs"]

[session]
lock_ttl_secs = 60
stall_after_secs = 300

[tokens]
default_read_budget = 4000
repo_map_budget = 1024

[[verify]]
name = "check"
run = ["cargo", "check", "--all-targets"]
timeout_secs = 600
"#,
        );
        let s = c.settings();
        assert_eq!(s.max_file_bytes, 2048);
        assert_eq!(s.extra_ignore, ["dist/", "*.min.js"]);
        assert_eq!(s.deny_read, [".env"]);
        assert_eq!((s.default_read_budget, s.repo_map_budget), (4000, 1024));
        assert_eq!(c.verify.len(), 1);
        assert_eq!(c.session.lock_ttl_secs, 60);
    }

    #[test]
    fn partial_section_keeps_other_defaults() {
        let c = parse("[tokens]\nrepo_map_budget = 512\n");
        assert_eq!(c.tokens.repo_map_budget, 512);
        assert_eq!(c.tokens.default_read_budget, 8000);
    }

    #[test]
    fn rejects_unknown_and_bad_verify() {
        for bad in [
            "[index]\nmax_file_byte = 1\n",
            "[unknown]\n",
            "[[verify]]\nname = \"x\"\nrun = []\n",
            "[[verify]]\nname = \"x\"\nrun = [\"a\"]\nshell = true\n",
        ] {
            assert!(ProjectConfig::parse(bad).is_err(), "{bad:?} harus ditolak");
        }
    }
}
