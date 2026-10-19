use std::fmt;

use crate::error::ParseValueError;

/// Status session. Teks dari [`SessionStatus::as_str`] disimpan di kolom
/// `sessions.status`; jangan mengganti nama.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionStatus {
    Active,
    /// Agen tidak terlihat melewati batas waktu; lock-nya sudah dilepas,
    /// tetapi perubahannya disimpan dan session bisa dilanjutkan.
    Stalled,
    InReview,
    Merged,
    Abandoned,
}

impl SessionStatus {
    pub const ALL: [Self; 5] = [
        Self::Active,
        Self::Stalled,
        Self::InReview,
        Self::Merged,
        Self::Abandoned,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stalled => "stalled",
            Self::InReview => "in_review",
            Self::Merged => "merged",
            Self::Abandoned => "abandoned",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, ParseValueError> {
        Self::ALL
            .into_iter()
            .find(|s| s.as_str() == raw)
            .ok_or_else(|| ParseValueError::new("status session", raw))
    }

    /// Session yang masih boleh diubah agennya.
    pub const fn is_editable(self) -> bool {
        matches!(self, Self::Active | Self::Stalled)
    }
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_roundtrip() {
        for s in SessionStatus::ALL {
            assert_eq!(SessionStatus::parse(s.as_str()), Ok(s));
        }
        assert!(SessionStatus::parse("Active").is_err());
        assert!(SessionStatus::Stalled.is_editable());
        assert!(!SessionStatus::InReview.is_editable());
    }
}
