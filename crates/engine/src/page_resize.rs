//! Thai edition: the sizes Resize pages offers (`Edit::ResizePages`): the common papers and "the
//! size most pages of this document have". Shared by the dialog and the `page_resize` tool so both
//! offer the same ones.

use pdfcraft_render::PageInfo;

/// The smallest and largest side a page may be resized to, in points.
pub use pdfcraft_edit::resize::{MAX_SIDE, MIN_SIDE};

/// The papers offered by name, in points, portrait, from the print settings' table
/// (`pdfcraft_print::PAPERS`). Short names a person or agent would type.
pub const NAMED: [(&str, &str); 6] =
    [("a4", "A4"), ("a3", "A3"), ("a5", "A5"), ("letter", "US Letter"), ("legal", "US Legal"), ("tabloid", "Tabloid")];

/// A paper by its short name (`a4`, `letter`…) or its full name in the print settings
/// (`US Letter`, `ARCH A (9 x 12 in)`…), in any case: its portrait size in points.
pub fn named(name: &str) -> Option<(f64, f64)> {
    let name = name.trim();
    let full = NAMED.iter().find(|(short, _)| short.eq_ignore_ascii_case(name)).map_or(name, |(_, full)| *full);
    pdfcraft_print::PAPERS.iter().find(|(n, _)| n.eq_ignore_ascii_case(full)).map(|(_, size)| *size)
}

/// The size (points, as shown) that most of `pages` have, sizes within half a point counted as
/// one; on a tie, the one the earliest page has. `None` for no pages.
pub fn most_common(pages: &[PageInfo]) -> Option<(f64, f64)> {
    let mut counts: Vec<((f64, f64), usize)> = Vec::new();
    for p in pages {
        let size = (f64::from(p.width), f64::from(p.height));
        if !(size.0.is_finite() && size.1.is_finite()) {
            continue;
        }
        match counts.iter_mut().find(|(s, _)| (s.0 - size.0).abs() <= 0.5 && (s.1 - size.1).abs() <= 0.5) {
            Some((_, n)) => *n += 1,
            None => counts.push((size, 1)),
        }
    }
    // `max_by_key` keeps the last of equals: scan from the end so the earliest wins a tie.
    counts.into_iter().rev().max_by_key(|(_, n)| *n).map(|(s, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(width: f32, height: f32) -> PageInfo {
        PageInfo { width, height, label: String::new(), crop: [0.0, 0.0, width, height], rotation: 0 }
    }

    #[test]
    fn papers_are_found_by_short_or_full_name() {
        assert_eq!(named("A4"), Some((595.28, 841.89)));
        assert_eq!(named(" letter "), Some((612.0, 792.0)));
        assert_eq!(named("us legal"), Some((612.0, 1008.0)));
        assert_eq!(named("ARCH A (9 x 12 in)"), Some((648.0, 864.0)));
        assert_eq!(named("B5"), None);
        for (short, _) in NAMED {
            assert!(named(short).is_some(), "{short}");
        }
    }

    #[test]
    fn the_size_most_pages_have_wins_and_the_earliest_breaks_a_tie() {
        let a4 = page(595.28, 841.89);
        let scan = page(1700.0, 2338.0);
        assert_eq!(most_common(&[scan.clone(), a4.clone(), page(595.0, 842.0), scan.clone()]).map(|s| s.0 as f32), Some(1700.0));
        assert_eq!(most_common(&[scan.clone(), a4.clone(), page(595.5, 842.0)]), Some((f64::from(595.28f32), f64::from(841.89f32))));
        assert_eq!(most_common(&[a4.clone(), scan.clone()]).map(|s| s.0 as f32), Some(595.28));
        assert_eq!(most_common(&[]), None);
    }
}
