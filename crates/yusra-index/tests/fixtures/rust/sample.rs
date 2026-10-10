//! Modul contoh untuk uji ekstraksi simbol. Tidak dikompilasi.

use std::fmt;
use crate::store::{Txn, Value};

mod helpers;

/// Batas percobaan.
pub const MAX_RETRY: u32 = 3;

static GREETING: &str = "halo";

/// Session login.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: u64,
    token: String,
}

pub enum State {
    Active,
    Closed,
}

impl Session {
    /// Buat session baru.
    pub fn new(id: u64) -> Self {
        let token = make_token(id);
        Self { id, token }
    }

    // komentar biasa, bukan doc
    pub fn refresh(&mut self) -> bool {
        self.token = make_token(self.id);
        true
    }
}

impl fmt::Display for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session {}", self.id)
    }
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("state")
    }
}

impl<T> From<Vec<T>> for State
where
    T: Clone,
{
    fn from(_: Vec<T>) -> Self {
        State::Active
    }
}

pub trait Repository {
    fn get(&self, key: &str) -> Option<String>;

    fn exists(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
}

/// Doc ini terpisah baris kosong, jadi tidak menempel.

fn make_token(id: u64) -> String {
    format!("t{id}")
}

pub mod auth {
    pub fn login() -> bool {
        super::make_token(1).is_empty()
    }

    #[cfg(unix)]
    fn platform() -> &'static str {
        "unix"
    }

    #[cfg(windows)]
    fn platform() -> &'static str {
        "windows"
    }
}
