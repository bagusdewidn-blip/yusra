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

#[derive(Debug)]
pub enum YusraError {
    InvalidPath { path: String, reason: PathRejection },
}

impl fmt::Display for YusraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath { path, reason } => {
                write!(f, "path tidak valid ({reason}): {path:?}")
            }
        }
    }
}

impl std::error::Error for YusraError {}

impl YusraError {
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidPath { .. } => ErrorCode::InvalidPath,
        }
    }
}
