use std::fmt;

/// Kode error yang dikirim ke agen. Bagian dari API publik: nama tidak boleh berubah.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    InvalidPath,
    PathDenied,
    NotFound,
    StaleBase,
    Locked,
    ParseError,
    PatchRejected,
    BudgetExceeded,
    SessionClosed,
    RebaseRequired,
    VerifyNotApproved,
    Internal,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidPath => "invalid_path",
            Self::PathDenied => "path_denied",
            Self::NotFound => "not_found",
            Self::StaleBase => "stale_base",
            Self::Locked => "locked",
            Self::ParseError => "parse_error",
            Self::PatchRejected => "patch_rejected",
            Self::BudgetExceeded => "budget_exceeded",
            Self::SessionClosed => "session_closed",
            Self::RebaseRequired => "rebase_required",
            Self::VerifyNotApproved => "verify_not_approved",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Alasan penolakan path. Ditampilkan di pesan error, bukan bagian kontrak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRejection {
    Empty,
    Absolute,
    ParentComponent,
    NulByte,
    Backslash,
    ControlChar,
}

impl fmt::Display for PathRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "path kosong",
            Self::Absolute => "path absolut tidak diizinkan",
            Self::ParentComponent => "komponen '..' tidak diizinkan",
            Self::NulByte => "mengandung byte NUL",
            Self::Backslash => "gunakan '/' sebagai pemisah, bukan '\\'",
            Self::ControlChar => "mengandung karakter kontrol",
        })
    }
}

/// Teks yang tidak bisa diubah menjadi tipe domain (hash, ID, nama enum).
///
/// Tidak punya [`ErrorCode`] sendiri: lapisan pemanggil yang memetakannya,
/// misalnya ID session yang tidak valid dilaporkan sebagai `not_found`.
/// Nilai asli dipotong agar input agen yang sangat panjang tidak ikut ke log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseValueError {
    kind: &'static str,
    value: String,
}

impl ParseValueError {
    const MAX_VALUE_CHARS: usize = 64;

    pub(crate) fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.chars().take(Self::MAX_VALUE_CHARS).collect(),
        }
    }

    pub fn kind(&self) -> &'static str {
        self.kind
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for ParseValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} tidak valid: {:?}", self.kind, self.value)
    }
}

impl std::error::Error for ParseValueError {}

#[derive(Debug)]
pub enum YusraError {
    InvalidPath {
        path: String,
        reason: PathRejection,
    },
    /// Cocok `deny_read`, berada di direktori internal (`.git`, `.yusra`),
    /// atau symlink yang keluar dari root.
    PathDenied {
        path: String,
    },
    NotFound {
        kind: &'static str,
        id: String,
    },
    /// Bug atau kegagalan I/O. Detail hanya untuk log; agen menerima `trace_id`.
    Internal {
        message: String,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

impl YusraError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            source: None,
        }
    }

    pub fn internal_with(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Internal {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidPath { .. } => ErrorCode::InvalidPath,
            Self::PathDenied { .. } => ErrorCode::PathDenied,
            Self::NotFound { .. } => ErrorCode::NotFound,
            Self::Internal { .. } => ErrorCode::Internal,
        }
    }
}

impl fmt::Display for YusraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath { path, reason } => {
                write!(f, "path tidak valid ({reason}): {path:?}")
            }
            Self::PathDenied { path } => write!(f, "akses ke path ditolak: {path:?}"),
            Self::NotFound { kind, id } => write!(f, "{kind} tidak ditemukan: {id:?}"),
            Self::Internal { message, .. } => write!(f, "kesalahan internal: {message}"),
        }
    }
}

impl std::error::Error for YusraError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Internal {
                source: Some(e), ..
            } => Some(e.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn codes_follow_variants() {
        let not_found = YusraError::NotFound {
            kind: "session",
            id: "x".to_owned(),
        };
        assert_eq!(not_found.code().as_str(), "not_found");
        let denied = YusraError::PathDenied {
            path: ".env".to_owned(),
        };
        assert_eq!(denied.code().as_str(), "path_denied");
        assert_eq!(YusraError::internal("x").code().as_str(), "internal");
    }

    #[test]
    fn internal_keeps_source() {
        let io = std::io::Error::other("disk penuh");
        let e = YusraError::internal_with("gagal menulis blob", io);
        assert_eq!(e.to_string(), "kesalahan internal: gagal menulis blob");
        match e.source() {
            Some(s) => assert_eq!(s.to_string(), "disk penuh"),
            None => panic!("source harus ada"),
        }
        assert!(YusraError::internal("x").source().is_none());
    }

    #[test]
    fn parse_value_error_truncates() {
        let long = "a".repeat(1000);
        let e = ParseValueError::new("hash", &long);
        assert_eq!(e.value().len(), ParseValueError::MAX_VALUE_CHARS);
        assert_eq!(e.kind(), "hash");
    }
}
