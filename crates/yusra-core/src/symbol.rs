use std::{fmt, ops::Range};

use crate::{error::ParseValueError, hash::Hash32, path::RelPath};

/// Jalur simbol di dalam satu file, segmen dipisah `::`, misalnya
/// `auth::Session::refresh`. [`SymbolPath::WHOLE_FILE`] berarti seluruh file.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolPath(pub String);

impl SymbolPath {
    pub const WHOLE_FILE: &'static str = "*";

    pub fn whole_file() -> Self {
        Self(Self::WHOLE_FILE.to_owned())
    }

    pub fn is_whole_file(&self) -> bool {
        self.0 == Self::WHOLE_FILE
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SymbolPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Teks dari [`SymbolKind::as_str`] disimpan di kolom `symbols.kind` dan
/// dipakai filter `kind` di `search_symbols`; jangan mengganti nama.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Impl,
    Module,
    Const,
}

impl SymbolKind {
    pub const ALL: [Self; 10] = [
        Self::Function,
        Self::Method,
        Self::Class,
        Self::Struct,
        Self::Enum,
        Self::Trait,
        Self::Interface,
        Self::Impl,
        Self::Module,
        Self::Const,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Interface => "interface",
            Self::Impl => "impl",
            Self::Module => "module",
            Self::Const => "const",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, ParseValueError> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == raw)
            .ok_or_else(|| ParseValueError::new("jenis simbol", raw))
    }
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub path: RelPath,
    pub symbol_path: SymbolPath,
    pub kind: SymbolKind,
    /// Termasuk doc comment yang menempel.
    pub byte_range: Range<usize>,
    pub signature: Option<String>,
    /// blake3 dari isi `byte_range`.
    pub body_hash: Hash32,
}

/// Definisi atau rujukan identifier, bahan repo map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    Def {
        name: String,
        file: RelPath,
        line: u32,
    },
    Ref {
        name: String,
        file: RelPath,
    },
}

impl Tag {
    pub fn name(&self) -> &str {
        match self {
            Self::Def { name, .. } | Self::Ref { name, .. } => name,
        }
    }

    pub fn file(&self) -> &RelPath {
        match self {
            Self::Def { file, .. } | Self::Ref { file, .. } => file,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_roundtrip() {
        for k in SymbolKind::ALL {
            assert_eq!(SymbolKind::parse(k.as_str()), Ok(k));
        }
        assert!(SymbolKind::parse("Function").is_err());
        assert!(SymbolKind::parse("").is_err());
    }

    #[test]
    fn whole_file() {
        assert!(SymbolPath::whole_file().is_whole_file());
        assert!(!SymbolPath("auth::Session".to_owned()).is_whole_file());
    }

    #[test]
    fn tag_accessors() {
        let file = match RelPath::parse("src/lib.rs") {
            Ok(p) => p,
            Err(e) => panic!("{e}"),
        };
        let tag = Tag::Ref {
            name: "refresh".to_owned(),
            file: file.clone(),
        };
        assert_eq!(tag.name(), "refresh");
        assert_eq!(tag.file(), &file);
    }
}
