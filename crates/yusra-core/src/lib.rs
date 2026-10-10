//! Tipe domain, error, dan trait bersama. Tidak bergantung pada crate lain di workspace.

mod error;
mod path;

pub use error::{ErrorCode, PathRejection, YusraError};
pub use path::RelPath;
