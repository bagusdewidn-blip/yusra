use crate::{path::RelPath, symbol::SymbolPath};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    /// `None` berarti file dihapus.
    Clean(Option<Vec<u8>>),
    Conflict(Vec<ConflictItem>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictItem {
    pub path: RelPath,
    pub symbol_path: Option<SymbolPath>,
    pub reason: ConflictReason,
}

/// Teks dari [`ConflictReason::as_str`] dikirim ke agen di `submit_for_review`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConflictReason {
    SameSymbol,
    Structural,
    ParseFailed,
    DeleteModify,
}

impl ConflictReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SameSymbol => "same_symbol",
            Self::Structural => "structural",
            Self::ParseFailed => "parse_failed",
            Self::DeleteModify => "delete_modify",
        }
    }
}

/// Merge tiga arah satu file. `None` pada input berarti file tidak ada di sisi itu.
pub trait Merger: Send + Sync {
    fn merge_file(
        &self,
        path: &RelPath,
        base: Option<&[u8]>,
        ours: Option<&[u8]>,
        theirs: Option<&[u8]>,
    ) -> MergeOutcome;
}
