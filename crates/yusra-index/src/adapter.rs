use tree_sitter::{Language, Parser, Tree};
use yusra_core::{RelPath, Symbol, Tag, YusraError};

/// Pengetahuan per bahasa: grammar dan cara membaca simbol, impor, serta tag.
///
/// Semua method menerima pohon yang sudah diparse dari `src` yang sama.
/// Pohon dengan node `ERROR` tetap diproses sebisanya; pemanggil yang
/// memutuskan apakah hasilnya dipakai.
pub trait LanguageAdapter: Send + Sync {
    /// Nama bahasa, misalnya `rust`. Disimpan di kolom `files.language`.
    fn id(&self) -> &'static str;

    /// Ekstensi file tanpa titik.
    fn extensions(&self) -> &'static [&'static str];

    fn language(&self) -> Language;

    /// Simbol dalam urutan sumber. `symbol_path` unik per file; nama ganda
    /// (misalnya fungsi dengan `#[cfg]` berbeda) diberi akhiran `#2`, `#3`.
    fn symbols(&self, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Symbol>;

    /// Spesifikasi impor mentah sesuai sintaks bahasa, belum diresolusi ke path.
    fn imports(&self, tree: &Tree, src: &[u8]) -> Vec<String>;

    /// Bahan repo map. `Tag::Def::line` dimulai dari 1.
    fn tags(&self, path: &RelPath, tree: &Tree, src: &[u8]) -> Vec<Tag>;
}

/// Adapter untuk semua bahasa yang diaktifkan lewat cargo feature.
#[allow(
    clippy::vec_init_then_push,
    unused_mut,
    reason = "setiap push bergantung pada cargo feature bahasa"
)]
pub fn builtin_adapters() -> Result<Vec<Box<dyn LanguageAdapter>>, YusraError> {
    let mut adapters: Vec<Box<dyn LanguageAdapter>> = Vec::new();
    #[cfg(feature = "lang-rust")]
    adapters.push(Box::new(crate::rust::RustAdapter::new()?));
    #[cfg(feature = "lang-typescript")]
    {
        use crate::typescript::TypeScriptAdapter;
        adapters.push(Box::new(TypeScriptAdapter::typescript()?));
        adapters.push(Box::new(TypeScriptAdapter::tsx()?));
        adapters.push(Box::new(TypeScriptAdapter::javascript()?));
    }
    #[cfg(feature = "lang-python")]
    adapters.push(Box::new(crate::python::PythonAdapter::new()?));
    Ok(adapters)
}

/// Adapter yang menangani `path` berdasarkan ekstensi, peka huruf besar-kecil.
pub fn adapter_for<'a>(
    adapters: &'a [Box<dyn LanguageAdapter>],
    path: &RelPath,
) -> Option<&'a dyn LanguageAdapter> {
    let file = path.components().last()?;
    let (_, ext) = file.rsplit_once('.')?;
    adapters
        .iter()
        .map(Box::as_ref)
        .find(|a| a.extensions().contains(&ext))
}

pub fn parse(adapter: &dyn LanguageAdapter, src: &[u8]) -> Result<Tree, YusraError> {
    let mut parser = Parser::new();
    parser.set_language(&adapter.language()).map_err(|e| {
        YusraError::internal_with(
            format!("grammar {} tidak cocok dengan tree-sitter", adapter.id()),
            e,
        )
    })?;
    parser.parse(src, None).ok_or_else(|| {
        YusraError::internal(format!("parser {} berhenti tanpa hasil", adapter.id()))
    })
}
