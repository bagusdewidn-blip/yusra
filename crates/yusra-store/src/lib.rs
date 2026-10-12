//! Penyimpanan state: SQLite WAL dan migrasi skema.
//!
//! Hanya crate ini yang bergantung pada `rusqlite`; crate lain memakai method
//! domain di [`Store`].

mod audit;
mod index;
mod store;

pub use audit::AuditEntry;
pub use index::FileRecord;
pub use store::Store;
