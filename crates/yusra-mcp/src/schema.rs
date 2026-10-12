//! Tipe request dan response tool MCP. Semua request menolak field yang
//! tidak dikenal.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use yusra_core::{ModelFamily, SymbolKind};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetProjectMapRequest {
    /// File yang sedang dikerjakan. Definisi yang dirujuknya diutamakan,
    /// dan file ini sendiri tidak ditampilkan di peta.
    #[serde(default)]
    pub focus_paths: Vec<String>,
    /// Identifier yang disebut dalam tugas, untuk diutamakan.
    #[serde(default)]
    pub mention_idents: Vec<String>,
    /// Anggaran token peta. Default `tokens.repo_map_budget`.
    pub max_tokens: Option<u32>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct GetProjectMapResponse {
    /// Per file: baris `nomor: teks` untuk setiap definisi terpilih.
    pub map: String,
    pub files_total: usize,
    pub files_shown: usize,
    pub tokens: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKindParam {
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

impl From<SymbolKindParam> for SymbolKind {
    fn from(k: SymbolKindParam) -> Self {
        match k {
            SymbolKindParam::Function => Self::Function,
            SymbolKindParam::Method => Self::Method,
            SymbolKindParam::Class => Self::Class,
            SymbolKindParam::Struct => Self::Struct,
            SymbolKindParam::Enum => Self::Enum,
            SymbolKindParam::Trait => Self::Trait,
            SymbolKindParam::Interface => Self::Interface,
            SymbolKindParam::Impl => Self::Impl,
            SymbolKindParam::Module => Self::Module,
            SymbolKindParam::Const => Self::Const,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchSymbolsRequest {
    /// Potongan `symbol_path`, tanpa membedakan huruf besar-kecil.
    pub query: String,
    pub kind: Option<SymbolKindParam>,
    /// Default 50, maksimum 200.
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SymbolHit {
    pub path: String,
    pub symbol_path: String,
    pub kind: String,
    pub signature: Option<String>,
    /// blake3 isi simbol, hex 64 karakter.
    pub hash: String,
}

/// `structuredContent` MCP harus berupa objek, jadi daftar dibungkus.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SearchSymbolsResponse {
    pub symbols: Vec<SymbolHit>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadFileRequest {
    pub path: String,
    /// Baca versi di dalam session. Belum ada session di versi ini.
    pub session_id: Option<String>,
    /// Sertakan signature definisi dari file lain yang dirujuk file ini.
    #[serde(default)]
    pub with_deps: bool,
    /// Default `tokens.default_read_budget`. Isi dipotong per baris.
    pub max_tokens: Option<u32>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ReadFileResponse {
    pub path: String,
    pub content: String,
    /// blake3 seluruh file, hex 64 karakter, termasuk bagian yang terpotong.
    pub file_hash: String,
    pub deps_context: Option<String>,
    pub truncated: bool,
    pub tokens: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadSymbolRequest {
    pub path: String,
    /// Misalnya `Session::refresh`; lihat `search_symbols`.
    pub symbol_path: String,
    /// Baca versi di dalam session. Belum ada session di versi ini.
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ReadSymbolResponse {
    pub source: String,
    pub hash: String,
    /// Dimulai dari 1, inklusif.
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelFamilyParam {
    OpenAi,
    Anthropic,
    Gemini,
    Generic,
}

impl From<ModelFamilyParam> for ModelFamily {
    fn from(m: ModelFamilyParam) -> Self {
        match m {
            ModelFamilyParam::OpenAi => Self::OpenAi,
            ModelFamilyParam::Anthropic => Self::Anthropic,
            ModelFamilyParam::Gemini => Self::Gemini,
            ModelFamilyParam::Generic => Self::Generic,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EstimateTokensRequest {
    pub text: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    pub model_family: ModelFamilyParam,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EstimateTokensResponse {
    pub tokens: u32,
    pub approximate: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_names_match_core() {
        for k in SymbolKind::ALL {
            let param: SymbolKindParam =
                match serde_json::from_value(serde_json::Value::from(k.as_str())) {
                    Ok(p) => p,
                    Err(e) => panic!("{k}: {e}"),
                };
            assert_eq!(SymbolKind::from(param), k);
        }
        for m in ModelFamily::ALL {
            let param: ModelFamilyParam =
                match serde_json::from_value(serde_json::Value::from(m.as_str())) {
                    Ok(p) => p,
                    Err(e) => panic!("{m}: {e}"),
                };
            assert_eq!(ModelFamily::from(param), m);
        }
    }

    #[test]
    fn unknown_fields_rejected() {
        let r: Result<ReadFileRequest, _> = serde_json::from_str(r#"{"path":"a.rs","extra":1}"#);
        assert!(r.is_err());
    }
}
