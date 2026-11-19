//! Edit text on a scanned page: an OCR word is invisible text over the scan, so rewriting it must
//! also hide the scanned word. The paper colour around the word becomes the cover drawn over it
//! ([`pdfcraft_edit::BlockStyle::cover`]) and the colour of its ink becomes the new text's.

use std::sync::Arc;

use pdfcraft_edit::TextBlock;
use pdfcraft_render::{PageInfo, PageRenderer, RenderConfig, RenderRequest, Tile};

/// Device pixels per point the word's surroundings are read at.
const SCALE: f32 = 3.0;
/// The most pixels read for one paragraph; a larger area is read at a lower resolution.
const MAX_PIXELS: f32 = 4_000_000.0;
/// How far around the word (points) the paper is read, at least.
const RING: f32 = 4.0;
/// How far past the word's box a piece of ink touching it may reach and still be the word's, in
/// heights of the box (Thai edition): the head of a letter, a tail, not the line above or below.
const REACH: f32 = 0.6;
/// How far above or below the box a detached mark may float and still be the word's, in heights.
const MARK_GAP: f32 = 0.35;

/// The paper colour, the ink colour (RGB 0–1), the patches over a scanned word's stray ink, and
/// the holes for other words' ink inside its box.
pub(crate) type WordCover = ([f64; 3], [f64; 3], Vec<[f64; 4]>, Vec<[f64; 4]>);

/// What covering invisible paragraph `b` needs: the paper and ink colours (RGB 0–1), the pieces
/// of the scanned word's own ink past its box, each a patch in user space (`[left, bottom, right,
/// top]`), and the pieces of other words' ink inside its box, each a hole. `others` are the boxes of the page's other paragraphs (user space): ink lying in
/// them is theirs. `None` when the paragraph is visible text or the page can't be read.
pub(crate) fn sample(cos: &pdfcraft_cos::Document, info: &PageInfo, page: usize, b: &TextBlock, others: &[[f64; 4]]) -> Option<WordCover> {
    if !b.invisible {
        return None;
    }
    // The word's box in view space (points, y down).
    let corners = [[b.rect[0], b.rect[1]], [b.rect[2], b.rect[1]], [b.rect[0], b.rect[3]], [b.rect[2], b.rect[3]]]
        .map(|[x, y]| info.user_to_view(x as f32, y as f32));
    let (x0, x1) = corners.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, z), c| (a.min(c[0]), z.max(c[0])));
    let (y0, y1) = corners.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, z), c| (a.min(c[1]), z.max(c[1])));
    let word = [x0, y0, x1, y1];
    let ring = RING.max((y1 - y0) * REACH + 1.0);
    let area = [x0 - ring, y0 - ring, x1 + ring, y1 + ring];
    if word.iter().any(|v| !v.is_finite()) || x1 <= x0 || y1 <= y0 {
        return None;
    }
    let pixels = (area[2] - area[0]) * (area[3] - area[1]) * SCALE * SCALE;
    let scale = if pixels > MAX_PIXELS { SCALE * (MAX_PIXELS / pixels).sqrt() } else { SCALE };
    let (page_w, page_h) = (info.width * scale, info.height * scale);
    let clamp = |v: f32, max: f32| v.clamp(0.0, max.max(0.0)).floor() as u32;
    let (tx0, ty0) = (clamp(area[0] * scale, page_w), clamp(area[1] * scale, page_h));
    let (tx1, ty1) = (clamp(area[2] * scale, page_w), clamp(area[3] * scale, page_h));
    if tx1 <= tx0 || ty1 <= ty0 {
        return None;
    }
    let tile = Tile { x: tx0, y: ty0, w: tx1 - tx0, h: ty1 - ty0 };
    // The working file as it is: written out only if the document holds unsaved changes, which
    // it doesn't between edits (each one reopens the file it wrote).
    let bytes = if cos.is_modified() {
        Arc::new(pdfcraft_cos::write_incremental(cos, &pdfcraft_cos::SaveOptions::default()).ok()?)
    } else {
        cos.bytes().clone()
    };
    // The document's true colours, whatever the reading colours are set to.
    let shot = PageRenderer::new(bytes, RenderConfig::default()).render(RenderRequest { page, scale, tile: Some(tile), ..Default::default() });
    if shot.error.is_some() {
        return None;
    }
    let pad = ((y1 - y0) * 0.1).clamp(0.5, 2.0);
    let (mut paper, mut ink) = (Vec::new(), Vec::new());
    for (i, px) in shot.rgba.as_chunks::<4>().0.iter().enumerate() {
        let (col, row) = (i % shot.width.max(1) as usize, i / shot.width.max(1) as usize);
        let x = (tx0 as f32 + col as f32 + 0.5) / scale;
        let y = (ty0 as f32 + row as f32 + 0.5) / scale;
        let rgb = [px[0], px[1], px[2]].map(|c| f64::from(c) / 255.0);
        if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
            ink.push(rgb);
        } else if x < x0 - pad || x > x1 + pad || y < y0 - pad || y > y1 + pad {
            paper.push(rgb);
        }
    }
    let luma = |c: &[f64; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let by_luma = |a: &[f64; 3], b: &[f64; 3]| luma(a).total_cmp(&luma(b));
    paper.sort_by(by_luma);
    ink.sort_by(by_luma);
    // The paper is the ring's median, so a table rule or a neighbouring letter in it doesn't count.
    let paper = paper.get(paper.len() / 2).copied().unwrap_or([1.0; 3]);
    // The ink is the average of the word's darkest pixels; a box without any is printed in black.
    let darkest = ink.get(..ink.len().div_ceil(8)).unwrap_or_default();
    let ink = if darkest.is_empty() {
        [0.0; 3]
    } else {
        let sum = darkest.iter().fold([0.0; 3], |s, c| [s[0] + c[0], s[1] + c[1], s[2] + c[2]]);
        sum.map(|v| v / darkest.len() as f64)
    };
    let clear = luma(&paper) - luma(&ink) >= 0.25;
    // The word's own letters past its box (Thai edition), as patches in user space.
    let (patches, holes) = if clear {
        let threshold = (luma(&paper) + luma(&ink)) / 2.0;
        let dark: Vec<bool> =
            shot.rgba.as_chunks::<4>().0.iter().map(|p| luma(&[p[0], p[1], p[2]].map(|c| f64::from(c) / 255.0)) < threshold).collect();
        let to_px = |v: f32, origin: u32| v * scale - origin as f32;
        let boxed = [to_px(x0, tx0), to_px(y0, ty0), to_px(x1, tx0), to_px(y1, ty0)];
        let theirs: Vec<[f32; 4]> = others
            .iter()
            .filter_map(|r| {
                let a = info.user_to_view(r[0] as f32, r[1] as f32);
                let z = info.user_to_view(r[2] as f32, r[3] as f32);
                let v = [a[0].min(z[0]), a[1].min(z[1]), a[0].max(z[0]), a[1].max(z[1])];
                v.iter().all(|c| c.is_finite()).then(|| [to_px(v[0], tx0), to_px(v[1], ty0), to_px(v[2], tx0), to_px(v[3], ty0)])
            })
            .collect();
        // Half a point round each, as the box's own padding.
        let pad = 0.5;
        let (stray, keep) = stray_ink(&dark, shot.width as usize, shot.height as usize, boxed, &theirs);
        let to_user = |pieces: Vec<[f32; 4]>, pad: f32| -> Vec<[f64; 4]> {
            pieces
                .into_iter()
                .map(|[l, t, r, b]| {
                    let view = |px: f32, origin: u32| (px + origin as f32) / scale;
                    let a = info.view_to_user(view(l, tx0) - pad, view(t, ty0) - pad);
                    let z = info.view_to_user(view(r, tx0) + pad, view(b, ty0) + pad);
                    [a[0].min(z[0]), a[1].min(z[1]), a[0].max(z[0]), a[1].max(z[1])].map(f64::from)
                })
                .filter(|r| r.iter().all(|v| v.is_finite()))
                .collect()
        };
        (to_user(stray, pad), to_user(keep, 0.25))
    } else {
        (Vec::new(), Vec::new())
    };
    let ink = if clear { ink } else { [0.0; 3] };
    Some((paper, ink, patches, holes))
}

/// The pieces of the word boxed by `word` (`[left, top, right, bottom]` in the `width` × `height`
/// grid of `dark` pixels) that reach past its box, each as its own box in pixels. The word's ink is
/// each piece touching its box that stays within reach of it (the head of a letter, a tail), and
/// each small piece floating just above or below it within its width (a tone mark, an upper
/// vowel). A piece reaching further belongs to a line above or below, and one lying mostly in a
/// box of `others` (the page's other words, in pixels) is that word's: the upper vowels of the
/// line below poke up into a word's box as its own lower vowels hang down. A long rule, a dotted
/// leader run into the word, and the marks of the words beside it are not its either.
/// Also the pieces of other words' ink inside the box, which its cover must leave showing.
fn stray_ink(dark: &[bool], width: usize, height: usize, word: [f32; 4], others: &[[f32; 4]]) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let [x0, y0, x1, y1] = word;
    let h = (y1 - y0).max(1.0);
    if width == 0 || height == 0 || dark.len() != width * height || !word.iter().all(|v| v.is_finite()) {
        return (Vec::new(), Vec::new());
    }
    let mut seen = vec![false; dark.len()];
    let mut stray = Vec::new();
    let mut keep = Vec::new();
    let mut stack = Vec::new();
    for start in 0..dark.len() {
        if seen.get(start).copied().unwrap_or(true) || !dark.get(start).copied().unwrap_or(false) {
            continue;
        }
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        stack.push(start);
        let (mut c0, mut r0, mut c1, mut r1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        let mut touches = false;
        while let Some(i) = stack.pop() {
            let (c, r) = (i % width, i / width);
            (c0, r0, c1, r1) = (c0.min(c), r0.min(r), c1.max(c), r1.max(r));
            let (cx, cy) = (c as f32 + 0.5, r as f32 + 0.5);
            touches |= cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1;
            for (dc, dr) in [(-1i64, -1i64), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (nc, nr) = (c as i64 + dc, r as i64 + dr);
                if nc < 0 || nr < 0 || nc as usize >= width || nr as usize >= height {
                    continue;
                }
                let n = nr as usize * width + nc as usize;
                if let Some(s) = seen.get_mut(n)
                    && !*s
                    && dark.get(n).copied().unwrap_or(false)
                {
                    *s = true;
                    stack.push(n);
                }
            }
        }
        let piece = [c0 as f32, r0 as f32, c1 as f32 + 1.0, r1 as f32 + 1.0];
        let middle = (piece[0] + piece[2]) / 2.0;
        let small = piece[3] - piece[1] <= h * REACH;
        let near = (piece[3] <= y0 && y0 - piece[3] <= h * MARK_GAP) || (piece[1] >= y1 && piece[1] - y1 <= h * MARK_GAP);
        // A piece much wider than the word is a rule or a dotted leader run into it, not a letter.
        let letter = piece[2] - piece[0] <= (x1 - x0) + 2.0 * h;
        let reach = h * REACH;
        let within = piece[0] >= x0 - reach && piece[1] >= y0 - reach && piece[2] <= x1 + reach && piece[3] <= y1 + reach;
        let past = piece[0] < x0 || piece[1] < y0 || piece[2] > x1 || piece[3] > y1;
        let centre = [middle, (piece[1] + piece[3]) / 2.0];
        let mine = centre[0] >= x0 && centre[0] <= x1 && centre[1] >= y0 && centre[1] <= y1;
        // Lying in another word's box, or floating nearer to a word above or below than to this
        // one (a tone mark of the line below, between the two), it is that word's.
        let gap = |b: &[f32; 4]| (b[1] - piece[3]).max(piece[1] - b[3]).max(0.0);
        let mine_gap = gap(&word);
        let elsewhere = !mine
            && others.iter().any(|o| {
                let over = centre[0] >= o[0] && centre[0] <= o[2];
                over && ((centre[1] >= o[1] && centre[1] <= o[3]) || gap(o) < mine_gap)
            });
        if letter && past && !elsewhere && ((touches && within) || (!touches && small && near && middle >= x0 && middle <= x1)) {
            stray.push(piece);
        }
        // Another word's ink reaching into this one's box (its padding included) stays shown.
        let pad = (h * 0.1).max(1.0);
        let inside = piece[0] < x1 + pad && piece[2] > x0 - pad && piece[1] < y1 + pad && piece[3] > y0 - pad;
        if elsewhere && inside {
            keep.push(piece);
        }
    }
    (stray, keep)
}

/// The colour (RGB 0–1) the page shows around `at` (view space: points from the displayed page's
/// top left), as rendered in its true colours: the median of a small square, so one stray dark
/// pixel or a letter's edge doesn't decide it (Thai edition: the cover box's eyedropper). `None`
/// when the point is off the page or the page can't be read.
pub(crate) fn colour_at(cos: &pdfcraft_cos::Document, info: &PageInfo, page: usize, at: [f32; 2]) -> Option<[f64; 3]> {
    const HALF: f32 = 1.5;
    const SCALE_AT: f32 = 2.0;
    if !(at[0].is_finite() && at[1].is_finite()) || at[0] < 0.0 || at[1] < 0.0 || at[0] > info.width || at[1] > info.height {
        return None;
    }
    let clamp = |v: f32, max: f32| v.clamp(0.0, max.max(0.0)).floor() as u32;
    let (page_w, page_h) = (info.width * SCALE_AT, info.height * SCALE_AT);
    let (tx0, ty0) = (clamp((at[0] - HALF) * SCALE_AT, page_w), clamp((at[1] - HALF) * SCALE_AT, page_h));
    let (tx1, ty1) = (clamp((at[0] + HALF) * SCALE_AT, page_w), clamp((at[1] + HALF) * SCALE_AT, page_h));
    if tx1 <= tx0 || ty1 <= ty0 {
        return None;
    }
    let tile = Tile { x: tx0, y: ty0, w: tx1 - tx0, h: ty1 - ty0 };
    let bytes = if cos.is_modified() {
        Arc::new(pdfcraft_cos::write_incremental(cos, &pdfcraft_cos::SaveOptions::default()).ok()?)
    } else {
        cos.bytes().clone()
    };
    let shot =
        PageRenderer::new(bytes, RenderConfig::default()).render(RenderRequest { page, scale: SCALE_AT, tile: Some(tile), ..Default::default() });
    if shot.error.is_some() {
        return None;
    }
    let mut px: Vec<[f64; 3]> = shot.rgba.as_chunks::<4>().0.iter().map(|p| [p[0], p[1], p[2]].map(|c| f64::from(c) / 255.0)).collect();
    let luma = |c: &[f64; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    px.sort_by(|a, b| luma(a).total_cmp(&luma(b)));
    px.get(px.len() / 2).copied()
}

#[cfg(test)]
mod tests {
    use super::stray_ink;

    /// A grid of `w` × `h` pixels with the given boxes (`[left, top, right, bottom)`) inked.
    fn grid(w: usize, h: usize, boxes: &[[usize; 4]]) -> Vec<bool> {
        let mut dark = vec![false; w * h];
        for &[l, t, r, b] in boxes {
            for y in t..b {
                for x in l..r {
                    dark[y * w + x] = true;
                }
            }
        }
        dark
    }

    /// The pieces of the word's own ink past its box are patched, each alone: a letter whose head
    /// crosses the box's left edge, and a tone mark floating just above it. A crowded row's
    /// neighbours are not: the line above (a letter whose tail dips into the box reaches too far up
    /// to be the word's), a mark beside the word rather than over it, and ink already inside the
    /// box needs no patch.
    #[test]
    fn the_words_own_stray_ink_is_patched_and_nothing_else() {
        let word = [40.0, 40.0, 140.0, 60.0];
        let dark = grid(
            200,
            120,
            &[
                [34, 42, 50, 58],   // a letter whose head crosses the box's left edge by 6
                [60, 33, 66, 37],   // a tone mark 3 above the box, over the word
                [80, 2, 130, 18],   // the line above
                [70, 10, 76, 45],   // a letter of the line above whose tail dips into the box
                [150, 33, 156, 37], // a mark beside the word, not over it
                [90, 42, 100, 58],  // ink inside the box
            ],
        );
        let (mut patches, kept) = stray_ink(&dark, 200, 120, word, &[]);
        assert!(kept.is_empty(), "no other word given: {kept:?}");
        patches.sort_by(|a, b| a[0].total_cmp(&b[0]));
        assert_eq!(patches, vec![[34.0, 42.0, 50.0, 58.0], [60.0, 33.0, 66.0, 37.0]]);
        // A rule through the word, far wider than it, run into its letters: no patch.
        let ruled = grid(200, 120, &[[30, 44, 120, 56], [0, 50, 200, 52]]);
        assert!(stray_ink(&ruled, 200, 120, word, &[]).0.is_empty());
        // No ink, or a grid of the wrong size: nothing.
        assert!(stray_ink(&grid(200, 120, &[]), 200, 120, word, &[]).0.is_empty());
        assert!(stray_ink(&[true; 10], 200, 120, word, &[]).0.is_empty());
        // The line below's upper vowel pokes up into the box: it lies mostly in that line's word, so
        // it is that word's and stays; the word's own lower vowel hanging out of the box is patched.
        let below = [40.0, 64.0, 140.0, 84.0];
        let crowded = grid(200, 120, &[[50, 44, 120, 56], [70, 58, 76, 72], [100, 57, 106, 63]]);
        let (patches, kept) = stray_ink(&crowded, 200, 120, word, &[below]);
        assert_eq!(patches, vec![[100.0, 57.0, 106.0, 63.0]]);
        assert_eq!(kept, vec![[70.0, 58.0, 76.0, 72.0]], "the line below's vowel is left showing");
        // Without the line below's box, its vowel would have been taken for the word's.
        assert_eq!(stray_ink(&crowded, 200, 120, word, &[]).0.len(), 2);
        // A tone mark of the line below floating between the two lines, nearer to that line's box
        // than to the word's, is that line's; one nearer to the word is the word's.
        let between = grid(200, 120, &[[50, 44, 120, 56], [80, 62, 84, 64], [110, 61, 114, 62]]);
        let lower = [40.0, 65.0, 140.0, 85.0];
        let (patches, kept) = stray_ink(&between, 200, 120, word, &[lower]);
        assert_eq!(patches, vec![[110.0, 61.0, 114.0, 62.0]]);
        assert!(kept.is_empty(), "the line below's mark lies clear of the box: {kept:?}");
        // The line below's tone mark reaching up into the word's box is left showing.
        let reaching = grid(200, 120, &[[50, 44, 120, 56], [80, 58, 84, 66]]);
        let (patches, kept) = stray_ink(&reaching, 200, 120, word, &[[40.0, 62.0, 140.0, 82.0]]);
        assert!(patches.is_empty(), "{patches:?}");
        assert_eq!(kept, vec![[80.0, 58.0, 84.0, 66.0]]);
    }
}
