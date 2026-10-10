use std::fmt;

use crate::error::{PathRejection, YusraError};

/// Path relatif terhadap root proyek, sudah dinormalisasi, separator `/`.
///
/// Satu-satunya cara membuat nilai ini dari input luar adalah [`RelPath::parse`].
/// Validasi ini murni sintaksis; pemeriksaan symlink terhadap filesystem
/// dilakukan terpisah saat path dibuka.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(String);

impl RelPath {
    pub fn parse(raw: &str) -> Result<Self, YusraError> {
        let reject = |reason| YusraError::InvalidPath {
            path: raw.to_owned(),
            reason,
        };

        if raw.contains('\0') {
            return Err(reject(PathRejection::NulByte));
        }
        if raw.chars().any(char::is_control) {
            return Err(reject(PathRejection::ControlChar));
        }
        if raw.contains('\\') {
            return Err(reject(PathRejection::Backslash));
        }
        if raw.starts_with('/') || has_drive_prefix(raw) {
            return Err(reject(PathRejection::Absolute));
        }

        let mut parts: Vec<&str> = Vec::new();
        for part in raw.split('/') {
            match part {
                "" | "." => {}
                ".." => return Err(reject(PathRejection::ParentComponent)),
                p => parts.push(p),
            }
        }
        if parts.is_empty() {
            return Err(reject(PathRejection::Empty));
        }
        Ok(Self(parts.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Komponen path, dari terluar ke terdalam.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `C:`, `c:foo`, dan sejenisnya. Daemon berjalan di Linux, tetapi agen bisa
/// mengirim path Windows; tolak agar tidak ditafsirkan sebagai nama file.
fn has_drive_prefix(raw: &str) -> bool {
    let b = raw.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str) -> String {
        match RelPath::parse(raw) {
            Ok(p) => p.as_str().to_owned(),
            Err(e) => panic!("{raw:?} seharusnya valid: {e}"),
        }
    }

    fn rejected(raw: &str) -> PathRejection {
        match RelPath::parse(raw) {
            Ok(p) => panic!("{raw:?} seharusnya ditolak, hasil {p}"),
            Err(YusraError::InvalidPath { reason, .. }) => reason,
        }
    }

    #[test]
    fn normalizes() {
        assert_eq!(ok("src/main.rs"), "src/main.rs");
        assert_eq!(ok("./src/main.rs"), "src/main.rs");
        assert_eq!(ok("src//lib/./mod.rs"), "src/lib/mod.rs");
        assert_eq!(ok("src/"), "src");
        assert_eq!(ok(".env.example"), ".env.example");
        assert_eq!(ok("a..b/c"), "a..b/c");
    }

    #[test]
    fn rejects_traversal() {
        assert_eq!(rejected("../etc/passwd"), PathRejection::ParentComponent);
        assert_eq!(rejected("a/../../b"), PathRejection::ParentComponent);
        assert_eq!(rejected("a/.."), PathRejection::ParentComponent);
    }

    #[test]
    fn rejects_absolute() {
        assert_eq!(rejected("/etc/passwd"), PathRejection::Absolute);
        assert_eq!(rejected("C:/Windows"), PathRejection::Absolute);
        assert_eq!(rejected("c:foo"), PathRejection::Absolute);
    }

    #[test]
    fn rejects_bad_bytes() {
        assert_eq!(rejected("a\0b"), PathRejection::NulByte);
        assert_eq!(rejected("a\nb"), PathRejection::ControlChar);
        assert_eq!(rejected("..\\etc"), PathRejection::Backslash);
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(rejected(""), PathRejection::Empty);
        assert_eq!(rejected("."), PathRejection::Empty);
        assert_eq!(rejected("./"), PathRejection::Empty);
    }

    #[test]
    fn error_code_is_stable() {
        let Err(e) = RelPath::parse("../x") else {
            panic!("harus error");
        };
        assert_eq!(e.code().as_str(), "invalid_path");
    }
}
