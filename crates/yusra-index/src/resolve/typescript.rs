//! Impor relatif TypeScript dan JavaScript (`./x`, `../x`).
//!
//! Spesifikasi paket (`react`, `node:fs`) dianggap pustaka luar. Alias dari
//! `paths`/`baseUrl` di `tsconfig.json` belum didukung.

use yusra_core::RelPath;

use super::{ProjectLayout, join, split_last, to_rel};

/// Urutan coba ekstensi, mengikuti resolusi `bundler` TypeScript.
const EXTENSIONS: [&str; 9] = ["ts", "tsx", "d.ts", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// Di TypeScript, `./x.js` boleh merujuk ke `x.ts` yang belum dikompilasi.
const SOURCE_FOR_OUTPUT: [(&str, &[&str]); 4] = [
    ("js", &["ts", "tsx"]),
    ("jsx", &["tsx"]),
    ("mjs", &["mts"]),
    ("cjs", &["cts"]),
];

pub(crate) fn resolve(from: &RelPath, spec: &str, layout: &ProjectLayout) -> Vec<RelPath> {
    let is_relative =
        spec == "." || spec == ".." || spec.starts_with("./") || spec.starts_with("../");
    if !is_relative {
        return Vec::new();
    }
    let (dir, _) = split_last(from.as_str());
    let Some(base) = join(dir, spec) else {
        return Vec::new();
    };
    candidates(&base)
        .into_iter()
        .find(|c| layout.contains(c))
        .and_then(|c| to_rel(&c))
        .into_iter()
        .collect()
}

fn candidates(base: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !base.is_empty() {
        out.push(base.to_owned());
        let (_, name) = split_last(base);
        if let Some((stem, ext)) = name.rsplit_once('.')
            && let Some((_, sources)) = SOURCE_FOR_OUTPUT.iter().find(|(e, _)| *e == ext)
        {
            let stem_path = &base[..base.len() - name.len() + stem.len()];
            out.extend(sources.iter().map(|s| format!("{stem_path}.{s}")));
        }
        out.extend(EXTENSIONS.iter().map(|e| format!("{base}.{e}")));
    }
    let index = super::child(base, "index");
    out.extend(EXTENSIONS.iter().map(|e| format!("{index}.{e}")));
    out
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
    fn tries_extensions_then_index() {
        let l = layout(&[
            "src/app.ts",
            "src/config.ts",
            "src/ui/index.tsx",
            "src/util.js",
            "src/types.d.ts",
            "src/style.css",
            "index.ts",
        ]);
        assert_eq!(resolved(&l, "src/app.ts", "./config"), ["src/config.ts"]);
        assert_eq!(resolved(&l, "src/app.ts", "./ui"), ["src/ui/index.tsx"]);
        assert_eq!(resolved(&l, "src/app.ts", "./util"), ["src/util.js"]);
        assert_eq!(resolved(&l, "src/app.ts", "./types"), ["src/types.d.ts"]);
        assert_eq!(resolved(&l, "src/app.ts", "./style.css"), ["src/style.css"]);
        assert_eq!(resolved(&l, "src/app.ts", ".."), ["index.ts"]);
    }

    #[test]
    fn js_extension_maps_to_ts_source() {
        let l = layout(&["src/app.ts", "src/config.ts", "src/view.tsx", "src/m.mts"]);
        assert_eq!(resolved(&l, "src/app.ts", "./config.js"), ["src/config.ts"]);
        assert_eq!(resolved(&l, "src/app.ts", "./view.jsx"), ["src/view.tsx"]);
        assert_eq!(resolved(&l, "src/app.ts", "./m.mjs"), ["src/m.mts"]);
    }

    #[test]
    fn bare_and_escaping_specs_are_external() {
        let l = layout(&["src/app.ts", "react.ts", "node_modules/x/index.js"]);
        assert!(resolved(&l, "src/app.ts", "react").is_empty());
        assert!(resolved(&l, "src/app.ts", "node:fs").is_empty());
        assert!(resolved(&l, "src/app.ts", "../../outside").is_empty());
        assert!(resolved(&l, "src/app.ts", "./missing").is_empty());
    }
}
