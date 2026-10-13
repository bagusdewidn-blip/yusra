//! Impor Python: relatif (`.models`, `..config`) dan absolut (`pkg.mod`).
//!
//! Impor absolut dicari dari root proyek lebih dulu, lalu dari direktori
//! file pengimpor naik ke atas: skrip yang mengimpor tetangganya, lalu tata
//! letak seperti `src/`. Modul yang tidak ada di proyek dianggap pustaka luar.

use yusra_core::RelPath;

use super::{ProjectLayout, ancestors, child, split_last, to_rel};

pub(crate) fn resolve(from: &RelPath, spec: &str, layout: &ProjectLayout) -> Vec<RelPath> {
    let dots = spec.chars().take_while(|&c| c == '.').count();
    let module: Vec<&str> = spec[dots..].split('.').filter(|s| !s.is_empty()).collect();
    let (dir, _) = split_last(from.as_str());

    let found = if dots > 0 {
        // `.` adalah paket file ini; setiap titik tambahan naik satu tingkat.
        let mut base = Some(dir);
        for _ in 1..dots {
            base = base.and_then(|b| (!b.is_empty()).then(|| split_last(b).0));
        }
        base.and_then(|b| module_file(layout, b, &module))
    } else if module.is_empty() {
        None
    } else {
        std::iter::once("")
            .chain(ancestors(from.as_str()).filter(|b| !b.is_empty()))
            .find_map(|b| module_file(layout, b, &module))
    };
    found.and_then(|f| to_rel(&f)).into_iter().collect()
}

/// `base/a/b.py`, `base/a/b.pyi`, atau `base/a/b/__init__.py(i)`.
fn module_file(layout: &ProjectLayout, base: &str, module: &[&str]) -> Option<String> {
    let path = module
        .iter()
        .fold(base.to_owned(), |acc, part| child(&acc, part));
    let mut candidates = Vec::new();
    if !module.is_empty() {
        candidates.push(format!("{path}.py"));
        candidates.push(format!("{path}.pyi"));
    }
    candidates.push(child(&path, "__init__.py"));
    candidates.push(child(&path, "__init__.pyi"));
    candidates.into_iter().find(|c| layout.contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(files: &[&str]) -> ProjectLayout {
        ProjectLayout::new(files.iter().filter_map(|f| to_rel(f)), |_| None)
    }

    fn resolved(layout: &ProjectLayout, from: &str, spec: &str) -> Vec<String> {
        let Some(from) = to_rel(from) else {
            panic!("path uji tidak valid: {from}");
        };
        resolve(&from, spec, layout)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn relative_imports() {
        let l = layout(&[
            "app/__init__.py",
            "app/api/__init__.py",
            "app/api/views.py",
            "app/api/models.py",
            "app/config/__init__.py",
            "app/helpers.pyi",
        ]);
        let from = "app/api/views.py";
        assert_eq!(resolved(&l, from, ".models"), ["app/api/models.py"]);
        assert_eq!(resolved(&l, from, "."), ["app/api/__init__.py"]);
        assert_eq!(resolved(&l, from, "..config"), ["app/config/__init__.py"]);
        assert_eq!(resolved(&l, from, "..helpers"), ["app/helpers.pyi"]);
        assert!(resolved(&l, from, "....too_far").is_empty());
        assert!(resolved(&l, from, ".missing").is_empty());
    }

    #[test]
    fn absolute_imports_prefer_root() {
        let l = layout(&[
            "pkg/__init__.py",
            "pkg/db.py",
            "pkg/sub/db.py",
            "pkg/sub/run.py",
            "src/lib/core.py",
            "src/lib/use.py",
        ]);
        assert_eq!(resolved(&l, "pkg/sub/run.py", "pkg.db"), ["pkg/db.py"]);
        assert_eq!(resolved(&l, "pkg/sub/run.py", "db"), ["pkg/sub/db.py"]);
        assert_eq!(resolved(&l, "pkg/sub/run.py", "pkg"), ["pkg/__init__.py"]);
        assert_eq!(
            resolved(&l, "src/lib/use.py", "lib.core"),
            ["src/lib/core.py"]
        );
        assert!(resolved(&l, "pkg/sub/run.py", "os").is_empty());
        assert!(resolved(&l, "pkg/sub/run.py", "logging.handlers").is_empty());
    }
}
