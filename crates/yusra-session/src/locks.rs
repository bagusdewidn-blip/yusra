//! Aturan tumpang-tindih lock per simbol.
//!
//! Dua lock pada file yang sama bertabrakan jika salah satunya seluruh file,
//! simbolnya sama, satu adalah induk yang lain menurut segmen `::`
//! (`Session` dan `Session::refresh`), atau rentang byte-nya beririsan di
//! versi file yang dilihat session peminta. Aturan rentang menangkap
//! hubungan yang tidak tampak dari nama, misalnya blok `impl Session` yang
//! memuat `Session::refresh`.

use std::ops::Range;

use yusra_core::{Symbol, SymbolPath};

/// `true` jika lock `a` dan `b` pada file yang sama bertabrakan. `view`
/// adalah simbol file menurut session yang meminta lock.
pub fn locks_overlap(a: &SymbolPath, b: &SymbolPath, view: &[Symbol]) -> bool {
    if a.is_whole_file() || b.is_whole_file() || a == b {
        return true;
    }
    if is_ancestor(a, b) || is_ancestor(b, a) {
        return true;
    }
    match (range_of(a, view), range_of(b, view)) {
        (Some(x), Some(y)) => x.start < y.end && y.start < x.end,
        _ => false,
    }
}

/// `true` jika lock `held` cukup untuk mengedit `target`.
pub fn lock_covers(held: &SymbolPath, target: &SymbolPath, view: &[Symbol]) -> bool {
    if held.is_whole_file() || held == target || is_ancestor(held, target) {
        return true;
    }
    match (range_of(held, view), range_of(target, view)) {
        (Some(h), Some(t)) => h.start <= t.start && t.end <= h.end,
        _ => false,
    }
}

/// `Session` adalah induk `Session::refresh`, tetapi bukan induk `Sessions`.
fn is_ancestor(parent: &SymbolPath, child: &SymbolPath) -> bool {
    child
        .as_str()
        .strip_prefix(parent.as_str())
        .is_some_and(|rest| rest.starts_with("::"))
}

fn range_of<'a>(symbol: &SymbolPath, view: &'a [Symbol]) -> Option<&'a Range<usize>> {
    view.iter()
        .find(|s| &s.symbol_path == symbol)
        .map(|s| &s.byte_range)
}

#[cfg(test)]
mod tests {
    use yusra_core::{Hash32, RelPath, SymbolKind};

    use super::*;

    fn sp(raw: &str) -> SymbolPath {
        SymbolPath(raw.to_owned())
    }

    fn view() -> Vec<Symbol> {
        let path = match RelPath::parse("src/lib.rs") {
            Ok(p) => p,
            Err(e) => panic!("{e}"),
        };
        let sym = |name: &str, range: Range<usize>| Symbol {
            path: path.clone(),
            symbol_path: sp(name),
            kind: SymbolKind::Function,
            byte_range: range,
            signature: None,
            body_hash: Hash32([0; 32]),
        };
        vec![
            sym("Session", 0..20),
            sym("impl Session", 30..100),
            sym("Session::new", 40..60),
            sym("Session::refresh", 60..90),
            sym("Sessions", 110..120),
            sym("login", 130..150),
        ]
    }

    #[test]
    fn overlap_rules() {
        let v = view();
        let o = |a, b| locks_overlap(&sp(a), &sp(b), &v);
        assert!(o("*", "login"));
        assert!(o("login", "*"));
        assert!(o("login", "login"));
        assert!(o("Session", "Session::refresh"), "induk menurut nama");
        assert!(
            o("Session::refresh", "impl Session"),
            "induk menurut rentang"
        );
        assert!(
            !o("Session::new", "Session::refresh"),
            "rentang bersebelahan"
        );
        assert!(!o("Session", "Sessions"));
        assert!(!o("login", "impl Session"));
        // Simbol yang belum ada di versi peminta hanya dinilai dari nama.
        assert!(!o("helper", "login"));
        assert!(o("Session::helper", "Session"));
    }

    #[test]
    fn cover_rules() {
        let v = view();
        let c = |held, target| lock_covers(&sp(held), &sp(target), &v);
        assert!(c("*", "login"));
        assert!(c("login", "login"));
        assert!(c("Session", "Session::refresh"));
        assert!(c("impl Session", "Session::refresh"));
        assert!(!c("Session::refresh", "impl Session"));
        assert!(!c("login", "*"));
        assert!(!c("Session::new", "Session::refresh"));
    }
}
