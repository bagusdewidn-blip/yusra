//! Impor Go lewat path modul di `go.mod`. Satu impor menunjuk satu paket,
//! yaitu semua file `.go` non-uji di direktorinya.
//!
//! Paket dari modul lain (pustaka standar, dependensi, `vendor/`) dianggap
//! pustaka luar.

use yusra_core::RelPath;

use super::{ProjectLayout, child, split_last, to_rel};

pub(crate) fn resolve(_from: &RelPath, spec: &str, layout: &ProjectLayout) -> Vec<RelPath> {
    // Modul terpanjang lebih dulu, jadi modul bersarang menang atas induknya.
    let Some(dir) = layout.go_modules().find_map(|(dir, module)| {
        let rest = spec.strip_prefix(module)?;
        match rest {
            "" => Some(dir.to_owned()),
            _ => rest.strip_prefix('/').map(|r| child(dir, r)),
        }
    }) else {
        return Vec::new();
    };
    layout
        .files_in(&dir)
        .filter(|f| {
            let (_, name) = split_last(f);
            name.ends_with(".go") && !name.ends_with("_test.go")
        })
        .filter_map(to_rel)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(files: &[(&str, &str)]) -> ProjectLayout {
        ProjectLayout::new(files.iter().filter_map(|(f, _)| to_rel(f)), |path| {
            files
                .iter()
                .find(|(f, _)| *f == path.as_str())
                .map(|(_, content)| content.as_bytes().to_vec())
        })
    }

    fn resolved(layout: &ProjectLayout, spec: &str) -> Vec<String> {
        let Some(from) = to_rel("cmd/app/main.go") else {
            panic!("path uji tidak valid");
        };
        resolve(&from, spec, layout)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn package_imports_resolve_to_directory_files() {
        let l = layout(&[
            ("go.mod", "module example.com/app\n"),
            ("cmd/app/main.go", ""),
            ("internal/store/db.go", ""),
            ("internal/store/tx.go", ""),
            ("internal/store/db_test.go", ""),
            ("internal/store/README.md", ""),
            ("internal/store/sql/q.go", ""),
            ("tools/go.mod", "module example.com/app/tools\n"),
            ("tools/gen.go", ""),
            ("root.go", ""),
        ]);
        assert_eq!(
            resolved(&l, "example.com/app/internal/store"),
            ["internal/store/db.go", "internal/store/tx.go"]
        );
        assert_eq!(resolved(&l, "example.com/app/tools"), ["tools/gen.go"]);
        assert_eq!(resolved(&l, "example.com/app"), ["root.go"]);
        assert!(resolved(&l, "example.com/application").is_empty());
        assert!(resolved(&l, "fmt").is_empty());
        assert!(resolved(&l, "github.com/x/y").is_empty());
    }
}
