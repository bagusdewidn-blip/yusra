use std::fmt;

use crate::error::ParseValueError;

/// Hash blake3 32 byte. Di JSON ditulis sebagai hex 64 karakter huruf kecil.
///
/// `yusra-core` tidak menghitung hash; nilai dibuat oleh crate yang memakai `blake3`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hash32(pub [u8; 32]);

impl Hash32 {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        self.to_string()
    }

    /// Huruf besar diterima; [`Hash32::to_hex`] selalu menghasilkan huruf kecil.
    pub fn from_hex(raw: &str) -> Result<Self, ParseValueError> {
        let invalid = || ParseValueError::new("hash", raw);
        if raw.len() != 64 {
            return Err(invalid());
        }
        let mut out = [0u8; 32];
        let (pairs, _) = raw.as_bytes().as_chunks::<2>();
        for (byte, &[hi, lo]) in out.iter_mut().zip(pairs) {
            let (Some(hi), Some(lo)) = (hex_digit(hi), hex_digit(lo)) else {
                return Err(invalid());
            };
            *byte = (hi << 4) | lo;
        }
        Ok(Self(out))
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    char::from(b).to_digit(16).map(|d| d as u8)
}

impl fmt::Display for Hash32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Hash32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash32({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn sample() -> Hash32 {
        let mut b = [0u8; 32];
        for (i, x) in b.iter_mut().enumerate() {
            *x = i as u8;
        }
        Hash32(b)
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(sample().to_hex(), HEX);
        assert_eq!(Hash32::from_hex(HEX), Ok(sample()));
        assert_eq!(Hash32::from_hex(&HEX.to_uppercase()), Ok(sample()));
        assert_eq!(format!("{:?}", sample()), format!("Hash32({HEX})"));
    }

    #[test]
    fn rejects_bad_hex() {
        for bad in ["", &HEX[..62], &format!("{HEX}00"), &HEX.replace('f', "g")] {
            assert!(Hash32::from_hex(bad).is_err(), "{bad:?} harus ditolak");
        }
        // 64 byte tetapi bukan ASCII: tidak boleh panic di batas karakter.
        let multibyte = "é".repeat(32);
        assert!(Hash32::from_hex(&multibyte).is_err());
    }
}
