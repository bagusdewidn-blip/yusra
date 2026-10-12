use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
};

use tiktoken_rs::CoreBPE;
use yusra_core::{ModelFamily, TokenCount, TokenEstimator};

/// Estimator berbasis `o200k_base` (tokenizer model OpenAI generasi
/// GPT-4o ke atas). Hasil untuk [`ModelFamily::OpenAi`] eksak; keluarga lain
/// memakai hitungan yang sama sebagai perkiraan karena tokenizer mereka tidak
/// tersedia secara lokal.
///
/// Tabel BPE baru dimuat pada pemanggilan pertama agar daemon yang belum
/// pernah menghitung token tidak menanggung memorinya.
#[derive(Default)]
pub struct Estimator {
    bpe: OnceLock<Option<CoreBPE>>,
}

impl Estimator {
    pub fn new() -> Self {
        Self::default()
    }

    fn bpe(&self) -> Option<&CoreBPE> {
        self.bpe
            .get_or_init(|| tiktoken_rs::o200k_base().ok())
            .as_ref()
    }
}

impl std::fmt::Debug for Estimator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Estimator")
            .field("loaded", &self.bpe.get().is_some())
            .finish()
    }
}

impl TokenEstimator for Estimator {
    fn count(&self, text: &str, family: ModelFamily) -> TokenCount {
        if text.is_empty() {
            return TokenCount::exact(0);
        }
        let Some(bpe) = self.bpe() else {
            return heuristic(text);
        };
        // encode_ordinary meng-unwrap hasil fancy-regex, yang bisa gagal pada
        // input patologis (batas backtrack). Teks dari agen tidak boleh
        // menjatuhkan handler.
        let Ok(n) = catch_unwind(AssertUnwindSafe(|| bpe.encode_ordinary(text).len())) else {
            return heuristic(text);
        };
        let n = u32::try_from(n).unwrap_or(u32::MAX);
        match family {
            ModelFamily::OpenAi => TokenCount::exact(n),
            ModelFamily::Anthropic | ModelFamily::Gemini | ModelFamily::Generic => {
                TokenCount::approx(n)
            }
        }
    }
}

/// Perkiraan kasar saat tokenizer tidak tersedia: satu token per tiga byte.
/// Kode rata-rata sekitar 3-4 byte per token, jadi angka ini cenderung
/// melebihkan, yang lebih aman untuk batas anggaran.
pub fn heuristic(text: &str) -> TokenCount {
    TokenCount::approx(u32::try_from(text.len().div_ceil(3)).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_is_exact_others_approximate() {
        let est = Estimator::new();
        assert_eq!(
            est.count("hello world", ModelFamily::OpenAi),
            TokenCount::exact(2)
        );
        for family in [
            ModelFamily::Anthropic,
            ModelFamily::Gemini,
            ModelFamily::Generic,
        ] {
            assert_eq!(est.count("hello world", family), TokenCount::approx(2));
        }
    }

    #[test]
    fn empty_is_zero() {
        let est = Estimator::new();
        assert_eq!(est.count("", ModelFamily::Anthropic), TokenCount::exact(0));
    }

    #[test]
    fn special_tokens_are_plain_text() {
        let est = Estimator::new();
        let n = est.count("<|endoftext|>", ModelFamily::OpenAi).tokens;
        assert!(
            n > 1,
            "token spesial harus dihitung sebagai teks biasa, dapat {n}"
        );
    }

    #[test]
    fn heuristic_rounds_up() {
        assert_eq!(heuristic("abcd"), TokenCount::approx(2));
        assert_eq!(heuristic("abc"), TokenCount::approx(1));
    }
}
