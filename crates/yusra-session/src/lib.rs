//! Aturan session yang tidak bergantung pada penyimpanan: tumpang-tindih
//! lock per simbol dan validasi edit terhadap isi file di memori.

mod edit;
mod locks;

pub use edit::{EditedFile, Editor, FileView, check_path};
pub use locks::{lock_covers, locks_overlap};
