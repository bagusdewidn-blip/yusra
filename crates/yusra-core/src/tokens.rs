use std::{fmt, iter::Sum, ops::Add};

use crate::error::ParseValueError;

/// Keluarga tokenizer. Teks dari [`ModelFamily::as_str`] adalah nilai
/// `model_family` di tool `estimate_tokens`; jangan mengganti nama.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    OpenAi,
    Anthropic,
    Gemini,
    Generic,
}

impl ModelFamily {
    pub const ALL: [Self; 4] = [Self::OpenAi, Self::Anthropic, Self::Gemini, Self::Generic];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Generic => "generic",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, ParseValueError> {
        Self::ALL
            .into_iter()
            .find(|m| m.as_str() == raw)
            .ok_or_else(|| ParseValueError::new("keluarga model", raw))
    }
}

impl fmt::Display for ModelFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Jumlah token. Penjumlahan bersifat saturating dan hasilnya perkiraan jika
/// salah satu operand perkiraan. `Default` adalah 0 eksak.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenCount {
    pub tokens: u32,
    pub approximate: bool,
}

impl TokenCount {
    pub const fn exact(tokens: u32) -> Self {
        Self {
            tokens,
            approximate: false,
        }
    }

    pub const fn approx(tokens: u32) -> Self {
        Self {
            tokens,
            approximate: true,
        }
    }
}

impl Add for TokenCount {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            tokens: self.tokens.saturating_add(rhs.tokens),
            approximate: self.approximate || rhs.approximate,
        }
    }
}

impl Sum for TokenCount {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), Add::add)
    }
}

pub trait TokenEstimator: Send + Sync {
    fn count(&self, text: &str, family: ModelFamily) -> TokenCount;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_roundtrip() {
        for m in ModelFamily::ALL {
            assert_eq!(ModelFamily::parse(m.as_str()), Ok(m));
        }
        assert!(ModelFamily::parse("OpenAI").is_err());
    }

    #[test]
    fn sum_marks_approximate() {
        let exact: TokenCount = [TokenCount::exact(3), TokenCount::exact(4)]
            .into_iter()
            .sum();
        assert_eq!(exact, TokenCount::exact(7));

        let mixed: TokenCount = [TokenCount::exact(3), TokenCount::approx(4)]
            .into_iter()
            .sum();
        assert_eq!(mixed, TokenCount::approx(7));

        let empty: TokenCount = std::iter::empty().sum();
        assert_eq!(empty, TokenCount::exact(0));
    }

    #[test]
    fn add_saturates() {
        let big = TokenCount::exact(u32::MAX) + TokenCount::exact(1);
        assert_eq!(big.tokens, u32::MAX);
    }
}
