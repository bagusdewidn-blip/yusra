use std::fmt;

use crate::error::ParseValueError;

/// Newtype `String` yang hanya bisa dibuat lewat `parse` dengan validator tertentu.
macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal, $normalize:path) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn parse(raw: &str) -> Result<Self, ParseValueError> {
                $normalize(raw)
                    .map(Self)
                    .ok_or_else(|| ParseValueError::new($kind, raw))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// ID objek commit Git: hex 40 karakter (SHA-1) atau 64 karakter (SHA-256), huruf kecil.
    CommitId,
    "commit",
    normalize_git_oid
);

string_id!(
    /// ID objek blob Git, format sama dengan [`CommitId`].
    BlobId,
    "blob",
    normalize_git_oid
);

string_id!(
    /// ULID session. Ikut membentuk path worktree, jadi validasi ini juga
    /// mencegah karakter path dari agen masuk ke filesystem.
    SessionId,
    "session",
    normalize_ulid
);

string_id!(
    /// ULID snapshot. Ikut membentuk nama ref `refs/yusra/snapshots/<id>`.
    SnapshotId,
    "snapshot",
    normalize_ulid
);

macro_rules! ulid_from_parts {
    ($name:ident) => {
        impl $name {
            /// ULID dari waktu (milidetik Unix, 48 bit bawah) dan 80 bit acak
            /// (bit di atasnya dibuang). Keacakan disediakan pemanggil.
            pub fn from_parts(unix_ms: u64, random: u128) -> Self {
                Self(encode_ulid(unix_ms, random))
            }
        }
    };
}

ulid_from_parts!(SessionId);
ulid_from_parts!(SnapshotId);

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn encode_ulid(unix_ms: u64, random: u128) -> String {
    let time = u128::from(unix_ms & ((1 << 48) - 1));
    let value = (time << 80) | (random & ((1 << 80) - 1));
    (0..26)
        .rev()
        .map(|i| char::from(CROCKFORD[((value >> (i * 5)) & 0x1f) as usize]))
        .collect()
}

fn normalize_git_oid(raw: &str) -> Option<String> {
    let ok = matches!(raw.len(), 40 | 64) && raw.bytes().all(|b| b.is_ascii_hexdigit());
    ok.then(|| raw.to_ascii_lowercase())
}

/// 26 karakter Crockford base32 (tanpa I, L, O, U). Huruf kecil diterima dan
/// dinormalisasi ke huruf besar, bentuk kanonis ULID. Karakter pertama paling
/// besar `7` karena ULID hanya 128 bit.
fn normalize_ulid(raw: &str) -> Option<String> {
    let s = raw.to_ascii_uppercase();
    let b = s.as_bytes();
    let ok = b.len() == 26
        && matches!(b[0], b'0'..=b'7')
        && b.iter().all(|c| {
            matches!(c, b'0'..=b'9' | b'A'..=b'H' | b'J' | b'K' | b'M' | b'N' | b'P'..=b'T' | b'V'..=b'Z')
        });
    ok.then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ULID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    const SHA1: &str = "e7ea02b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6";

    #[test]
    fn accepts_git_oids() {
        assert_eq!(
            CommitId::parse(SHA1).map(|c| c.to_string()),
            Ok(SHA1.to_owned())
        );
        let upper = SHA1.to_uppercase();
        assert_eq!(
            BlobId::parse(&upper).map(|b| b.to_string()),
            Ok(SHA1.to_owned())
        );
        let sha256 = "ab".repeat(32);
        assert!(CommitId::parse(&sha256).is_ok());
    }

    #[test]
    fn rejects_bad_git_oids() {
        for bad in [
            "",
            "HEAD",
            "main",
            &SHA1[..39],
            &format!("{SHA1}0"),
            &SHA1.replace('e', "x"),
        ] {
            assert!(CommitId::parse(bad).is_err(), "{bad:?} harus ditolak");
        }
    }

    #[test]
    fn accepts_ulids() {
        assert_eq!(
            SessionId::parse(ULID).map(|s| s.to_string()),
            Ok(ULID.to_owned())
        );
        let lower = ULID.to_lowercase();
        assert_eq!(
            SnapshotId::parse(&lower).map(|s| s.to_string()),
            Ok(ULID.to_owned())
        );
    }

    #[test]
    fn ulid_from_parts_is_canonical_and_sortable() {
        let a = SessionId::from_parts(1_700_000_000_000, 0);
        let b = SessionId::from_parts(1_700_000_000_001, u128::MAX);
        assert_eq!(SessionId::parse(a.as_str()), Ok(a.clone()));
        assert_eq!(SessionId::parse(b.as_str()), Ok(b.clone()));
        assert!(a < b, "urutan teks mengikuti waktu");
        assert_eq!(SnapshotId::from_parts(0, 0).as_str(), "0".repeat(26));
        assert_eq!(
            SnapshotId::from_parts(u64::MAX, u128::MAX).as_str(),
            format!("7{}", "Z".repeat(25))
        );
    }

    #[test]
    fn rejects_bad_ulids() {
        let overflow = format!("8{}", &ULID[1..]);
        let with_i = format!("{}I", &ULID[..25]);
        for bad in [
            "",
            "../x",
            "refs/heads/main",
            &ULID[..25],
            &overflow,
            &with_i,
        ] {
            assert!(SessionId::parse(bad).is_err(), "{bad:?} harus ditolak");
        }
        match SnapshotId::parse("a/b") {
            Err(e) => assert_eq!(e.kind(), "snapshot"),
            Ok(id) => panic!("harus ditolak, hasil {id}"),
        }
    }
}
