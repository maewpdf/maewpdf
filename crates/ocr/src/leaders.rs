//! Thai edition: dotted leaders ("เขียนที่........", "อายุ.......ปี"), found in the page image
//! itself. The text detector takes a short word squeezed between two leaders for part of the
//! dots and drops it. So the dots are found here first, and the page is read with them whited
//! out. They stay as they are in the scan: drawn as text they were heavier than the scan's dots.
//!
//! A leader is a row of small, near-square ink specks of one size at one height, evenly spaced.
//! Tone marks and vowels above a Thai line are small too, but differ in shape, size and spacing,
//! so they don't make a run.

/// Fewest specks that make a leader: fewer could be an ellipsis or a few marks that line up.
const MIN_DOTS: usize = 8;
/// Most specks collected on one page; past it the page is read without looking for leaders.
const MAX_SPECKS: usize = 200_000;

/// A dark connected component: its box (`right` and `bottom` exclusive) and its pixel count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Speck {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
    area: usize,
}

impl Speck {
    fn width(&self) -> usize {
        self.right - self.left
    }
    fn height(&self) -> usize {
        self.bottom - self.top
    }
    /// Twice the centre's height: whole numbers, no rounding.
    fn middle2(&self) -> usize {
        self.top + self.bottom
    }
}

/// The pixels of the leaders in the grey image `grey` (`w` × `h`, row by row). `None` when there
/// are none, or the page has too many specks to look at.
pub(crate) fn find(grey: &[u8], w: usize, h: usize) -> Option<Vec<bool>> {
    if w == 0 || h == 0 || grey.len() != w.checked_mul(h)? {
        return None;
    }
    let dark = otsu(grey);
    // A dot is at most about a 200th of the page's longer side (12 px for A4 at 300 dpi).
    let biggest = (w.max(h) / 200).clamp(3, 40);
    let specks = specks(grey, w, h, dark, biggest)?;
    let runs: Vec<Vec<Speck>> = runs(specks).into_iter().filter(|r| r.len() >= MIN_DOTS).collect();
    if runs.is_empty() {
        return None;
    }
    let mut mask = vec![false; w * h];
    for s in runs.iter().flatten() {
        // The speck's box and a pixel round it: the dot's soft edge goes too.
        for y in s.top.saturating_sub(1)..(s.bottom + 1).min(h) {
            for x in s.left.saturating_sub(1)..(s.right + 1).min(w) {
                if let Some(m) = mask.get_mut(y * w + x) {
                    *m = true;
                }
            }
        }
    }
    Some(mask)
}

/// The grey level that splits ink from paper (Otsu's method): darker is ink.
fn otsu(grey: &[u8]) -> u8 {
    let mut histogram = [0u64; 256];
    for &g in grey {
        histogram[usize::from(g)] += 1;
    }
    let total = grey.len() as f64;
    let sum: f64 = histogram.iter().enumerate().map(|(g, n)| g as f64 * *n as f64).sum();
    let (mut below, mut below_sum, mut best, mut split) = (0.0f64, 0.0f64, -1.0f64, 128u8);
    for (g, n) in histogram.iter().enumerate() {
        below += *n as f64;
        below_sum += g as f64 * *n as f64;
        let above = total - below;
        if below == 0.0 || above == 0.0 {
            continue;
        }
        let between = below * above * (below_sum / below - (sum - below_sum) / above).powi(2);
        if between > best {
            best = between;
            split = u8::try_from(g).unwrap_or(u8::MAX);
        }
    }
    split
}

/// The 8-connected dark components no bigger than `biggest` a side that are dot-shaped: about as
/// wide as tall and mostly filled. `None` past [`MAX_SPECKS`].
fn specks(grey: &[u8], w: usize, h: usize, dark: u8, biggest: usize) -> Option<Vec<Speck>> {
    let is_dark = |at: usize| grey.get(at).is_some_and(|g| *g <= dark);
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut out = Vec::new();
    for start in 0..w * h {
        if seen[start] || !is_dark(start) {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut l, mut t, mut r, mut b, mut area) = (start % w, start / w, start % w, start / w, 0usize);
        while let Some(at) = stack.pop() {
            let (x, y) = (at % w, at / w);
            (l, t, r, b, area) = (l.min(x), t.min(y), r.max(x), b.max(y), area + 1);
            for ny in y.saturating_sub(1)..(y + 2).min(h) {
                for nx in x.saturating_sub(1)..(x + 2).min(w) {
                    let n = ny * w + nx;
                    if !seen[n] && is_dark(n) {
                        seen[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        let s = Speck { left: l, top: t, right: r + 1, bottom: b + 1, area };
        let (sw, sh) = (s.width(), s.height());
        let round = sw.max(sh) * 2 <= sw.min(sh) * 3 + 2 && area * 10 >= sw * sh * 4;
        if sw <= biggest && sh <= biggest && area >= 2 && round {
            out.push(s);
            if out.len() > MAX_SPECKS {
                return None;
            }
        }
    }
    Some(out)
}

/// The specks as rows: each speck joins the row whose last speck is just before it, at its
/// height and of its size; then each row is cut where its spacing or sizes stop being even.
fn runs(mut specks: Vec<Speck>) -> Vec<Vec<Speck>> {
    specks.sort_by_key(|s| (s.left, s.top));
    let mut rows: Vec<Vec<Speck>> = Vec::new();
    // The rows still open: their last speck may yet be followed.
    let mut open: Vec<usize> = Vec::new();
    for s in specks {
        let size = s.width().max(s.height());
        open.retain(|&i| rows.get(i).and_then(|r| r.last()).is_some_and(|last| last.right + 4 * size + 2 >= s.left));
        let follows = |last: &Speck| {
            let size_last = last.width().max(last.height());
            last.right <= s.left
                && s.left - last.right <= 3 * size.max(size_last) + 2
                && last.middle2().abs_diff(s.middle2()) <= size.max(size_last).max(2)
                && size * 2 >= size_last
                && size_last * 2 >= size
        };
        let best = open
            .iter()
            .copied()
            .filter(|&i| rows.get(i).and_then(|r| r.last()).is_some_and(follows))
            .min_by_key(|&i| rows.get(i).and_then(|r| r.last()).map_or(usize::MAX, |last| s.left - last.right));
        match best.and_then(|i| rows.get_mut(i)) {
            Some(row) => row.push(s),
            None => {
                open.push(rows.len());
                rows.push(vec![s]);
            }
        }
    }
    rows.into_iter().flat_map(even).collect()
}

/// `row` cut into the stretches whose gaps and sizes are even: within half again of the row's
/// middle gap and size.
fn even(row: Vec<Speck>) -> Vec<Vec<Speck>> {
    let median = |mut v: Vec<usize>| -> usize {
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(0)
    };
    let gap = median(row.windows(2).filter_map(|p| Some(p.get(1)?.left.saturating_sub(p.first()?.right))).collect());
    let size = median(row.iter().map(|s| s.width().max(s.height())).collect());
    let near = |v: usize, m: usize| v * 2 <= m * 3 + 2 && v * 3 + 2 >= m * 2;
    let mut out: Vec<Vec<Speck>> = vec![Vec::new()];
    for s in row {
        // A speck of another size ends the stretch and is no dot; an uneven gap starts a new one.
        if !near(s.width().max(s.height()), size) {
            out.push(Vec::new());
            continue;
        }
        if out.last().and_then(|r| r.last()).is_some_and(|last| !near(s.left.saturating_sub(last.right), gap)) {
            out.push(Vec::new());
        }
        if let Some(r) = out.last_mut() {
            r.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white page with dark squares `side` px at the given top-left corners.
    fn page(w: usize, h: usize, squares: &[(usize, usize, usize)]) -> Vec<u8> {
        let mut grey = vec![255u8; w * h];
        for &(x0, y0, side) in squares {
            for y in y0..y0 + side {
                for x in x0..x0 + side {
                    grey[y * w + x] = 0;
                }
            }
        }
        grey
    }

    /// Twelve even dots are a leader; their pixels are masked, the word beside them is not.
    #[test]
    fn an_even_row_of_dots_is_a_leader_and_a_word_is_not() {
        let mut squares: Vec<(usize, usize, usize)> = (0..12).map(|i| (60 + i * 8, 40, 3)).collect();
        squares.push((10, 30, 20)); // a word's blot, too big to be a dot
        let grey = page(200, 80, &squares);
        let mask = find(&grey, 200, 80).unwrap();
        assert!((0..12).all(|i| mask[41 * 200 + 61 + i * 8]), "every dot is masked");
        assert!(!mask[35 * 200 + 15], "the word is not");
        assert_eq!(mask.iter().filter(|m| **m).count(), 12 * 5 * 5, "each dot and a pixel round it, nothing else");
    }

    /// Too few dots, dots at different heights, and dots of very different sizes or spacing are
    /// not leaders: marks above a Thai line look like that.
    #[test]
    fn uneven_or_short_rows_are_not_leaders() {
        let short: Vec<_> = (0..5).map(|i| (20 + i * 8, 40, 3)).collect();
        assert!(find(&page(200, 80, &short), 200, 80).is_none());
        let wavy: Vec<_> = (0..12).map(|i| (20 + i * 8, 30 + (i % 2) * 12, 3)).collect();
        assert!(find(&page(200, 80, &wavy), 200, 80).is_none());
        let spaced: Vec<_> = [0, 7, 30, 37, 70, 77, 110, 117, 150, 157, 190].iter().map(|x| (x + 4, 40, 3)).collect();
        assert!(find(&page(220, 80, &spaced), 220, 80).is_none());
    }

    /// Nothing to find, or a buffer that isn't the size given, is no leader and no panic.
    #[test]
    fn a_blank_or_wrong_sized_page_has_none() {
        assert!(find(&[255; 100], 10, 10).is_none());
        assert!(find(&[0; 5], 10, 10).is_none());
        assert!(find(&[], 0, 0).is_none());
    }

    #[test]
    fn otsu_splits_ink_from_paper() {
        let grey: Vec<u8> = [vec![20u8; 30], vec![230u8; 70]].concat();
        let t = otsu(&grey);
        assert!((20..230).contains(&t), "{t}");
    }
}
