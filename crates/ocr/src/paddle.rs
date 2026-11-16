//! Thai edition: Thai text recognition (Apache-2.0 models): PaddleOCR's PP-OCRv3 text detection
//! and OpenThai 2.0 OCR Text's Thai line recogniser (iApp's fine-tune of PP-OCRv6 tiny; until
//! 2026-11-16 PaddleOCR's PP-OCRv5 Thai), in ONNX form, run in this process by the same engine as
//! the Latin models (rten). No page leaves the machine.
//!
//! Detection gives each pixel a probability of being text: the regions above [`BINARY`] are the
//! lines. Recognition reads one line at a time: each time step of its output is a character or
//! the CTC blank, repeats merged.

use std::path::{Path, PathBuf};

use rten::Model;
use rten_tensor::prelude::*;
use rten_tensor::{NdTensor, NdTensorView};

use crate::{Line, OcrError, Word};

pub const DETECTION_MODEL: &str = "paddle-text-detection.onnx";
pub const RECOGNITION_MODEL: &str = "openthai-thai-recognition.onnx";
pub const DICTIONARY: &str = "openthai-thai-dict.txt";

/// Longest side, in pixels, of the picture detection sees; a side is a multiple of [`STRIDE`].
const MAX_SIDE: usize = 1920;
/// The second, smaller look detection takes, for the lines the first missed.
const SMALL_SIDE: usize = 960;
/// Two lines are one text read twice when more than this share of either lies on the other.
const COVERED: f32 = 0.3;
const STRIDE: usize = 32;
/// A pixel is text from this probability.
const BINARY: f32 = 0.3;
/// A line is read when the recogniser's mean certainty of its characters is at least this
/// (PaddleOCR's `drop_score`).
const DROP_SCORE: f32 = 0.5;
/// A region is a line of text from this mean probability.
const REGION: f32 = 0.5;
/// How far a region is grown, as PaddleOCR's `unclip_ratio`: the model finds a line's core.
const UNCLIP: f32 = 1.6;
/// The recognition model's line height, and the widest line it is given.
const LINE_HEIGHT: usize = 48;
const MAX_LINE_WIDTH: usize = 3200;
/// Most regions read on one page.
const MAX_REGIONS: usize = 4000;

/// Where the Thai model files are.
#[derive(Clone, Debug, PartialEq)]
pub struct ThaiModels {
    pub detection: PathBuf,
    pub recognition: PathBuf,
    pub dictionary: PathBuf,
}

impl ThaiModels {
    /// The Thai models in `dir`, when all three are there.
    pub fn in_dir(dir: &Path) -> Option<ThaiModels> {
        let models = ThaiModels { detection: dir.join(DETECTION_MODEL), recognition: dir.join(RECOGNITION_MODEL), dictionary: dir.join(DICTIONARY) };
        (models.detection.is_file() && models.recognition.is_file() && models.dictionary.is_file()).then_some(models)
    }
}

/// A loaded Thai recogniser.
pub struct ThaiOcr {
    detection: Model,
    recognition: Model,
    /// The characters by class; class 0 is the CTC blank.
    alphabet: Vec<String>,
}

fn failed(e: &dyn std::fmt::Display) -> OcrError {
    OcrError::Recognize(e.to_string())
}

impl ThaiOcr {
    pub fn load(models: &ThaiModels) -> Result<ThaiOcr, OcrError> {
        let load = |p: &Path| Model::load_file(p).map_err(|e| OcrError::Load(p.display().to_string(), e.to_string()));
        let text = std::fs::read_to_string(&models.dictionary).map_err(|e| OcrError::Load(models.dictionary.display().to_string(), e.to_string()))?;
        Ok(ThaiOcr { detection: load(&models.detection)?, recognition: load(&models.recognition)?, alphabet: alphabet(&text) })
    }

    /// Recognise the text in an RGBA, RGB or grey image.
    pub fn recognize(&self, pixels: &[u8], width: u32, height: u32) -> Result<Vec<Line>, OcrError> {
        let (w, h) = (width as usize, height as usize);
        let area = w.checked_mul(h).filter(|a| *a > 0).ok_or(OcrError::EmptyImage)?;
        let channels = [4, 3, 1].into_iter().find(|c| area.checked_mul(*c) == Some(pixels.len())).ok_or(OcrError::EmptyImage)?;
        // One byte order for everything after this: the models were trained on BGR pictures.
        let original = |x: usize, y: usize| -> [u8; 3] {
            let at = (y * w + x) * channels;
            match pixels.get(at..at + channels) {
                Some([grey]) => [*grey; 3],
                Some([r, g, b, ..]) => [*b, *g, *r],
                _ => [255; 3],
            }
        };
        // Dotted leaders are whited out before reading (Thai edition), so the words between them
        // stand alone. The dots stay in the scan; they are not text.
        let grey: Vec<u8> = (0..area).map(|at| original(at % w, at / w).iter().map(|c| u32::from(*c)).sum::<u32>() / 3).map(|g| g as u8).collect();
        let leaders = crate::leaders::find(&grey, w, h);
        let bgr = |x: usize, y: usize| -> [u8; 3] {
            if leaders.as_ref().is_some_and(|mask| mask.get(y * w + x).copied().unwrap_or(false)) { [255; 3] } else { original(x, y) }
        };
        // The detector is sure of different lines at different sizes: a page made small finds
        // some short words the full-size look misses, and splits others differently. Both looks
        // are read; a small-look line that is new is added, and one that is a full-size line
        // again replaces it only when the recogniser is surer of it.
        let first = self.regions(&bgr, w, h, MAX_SIDE)?;
        let second = if w.max(h) > SMALL_SIDE { self.regions(&bgr, w, h, SMALL_SIDE)? } else { Vec::new() };
        let mut kept: Vec<Option<([f32; 4], Line, f32)>> = Vec::new();
        for region in first {
            if let Some((line, sure)) = self.read_line(&bgr, region)? {
                kept.push(Some((region, line, sure)));
            }
        }
        let full_size = kept.len();
        for region in second {
            if kept.len() >= MAX_REGIONS {
                break;
            }
            let Some((line, sure)) = self.read_line(&bgr, region)? else { continue };
            let clashes: Vec<usize> =
                (0..full_size).filter(|i| kept.get(*i).and_then(Option::as_ref).is_some_and(|(a, _, _)| same_text(a, &region))).collect();
            let surer = clashes.iter().all(|i| kept.get(*i).and_then(Option::as_ref).is_none_or(|(_, _, s)| *s < sure));
            if surer {
                for i in clashes {
                    if let Some(k) = kept.get_mut(i) {
                        *k = None;
                    }
                }
                kept.push(Some((region, line, sure)));
            }
        }
        let read: Vec<Line> = kept.into_iter().flatten().map(|(_, line, _)| line).collect();
        Ok(reading_order(read))
    }

    /// The lines of text on the page, as boxes in image pixels `[left, top, right, bottom]`.
    fn regions(&self, bgr: &dyn Fn(usize, usize) -> [u8; 3], w: usize, h: usize, max_side: usize) -> Result<Vec<[f32; 4]>, OcrError> {
        const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
        const STD: [f32; 3] = [0.229, 0.224, 0.225];
        let scale = (max_side as f32 / w.max(h) as f32).min(1.0);
        let side = |v: usize| (((v as f32 * scale) as usize).div_ceil(STRIDE) * STRIDE).max(STRIDE);
        let (sw, sh) = (side(w), side(h));
        let mut input = NdTensor::<f32, 4>::zeros([1, 3, sh, sw]);
        for y in 0..sh {
            let (y0, y1) = span(y, h, sh);
            for x in 0..sw {
                let (x0, x1) = span(x, w, sw);
                let mut sum = [0u32; 3];
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let px = bgr(sx, sy);
                        for c in 0..3 {
                            sum[c] += u32::from(px[c]);
                        }
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)).max(1) as f32;
                for c in 0..3 {
                    input[[0, c, y, x]] = (sum[c] as f32 / n / 255.0 - MEAN[c]) / STD[c];
                }
            }
        }
        let output: NdTensor<f32, 4> =
            self.detection.run_one(input.view().into(), None).map_err(|e| failed(&e))?.try_into().map_err(|e| failed(&e))?;
        let [_, _, oh, ow] = output.shape();
        let map = |x: usize, y: usize| output[[0, 0, y, x]];
        let (sx, sy) = (w as f32 / ow as f32, h as f32 / oh as f32);
        Ok(attach_marks(
            components(&map, ow, oh)
                .into_iter()
                .take(MAX_REGIONS)
                .map(|[l, t, r, b]| {
                    let (bw, bh) = (r - l, b - t);
                    let grow = (bw * bh * UNCLIP / (2.0 * (bw + bh)).max(1.0)).max(1.0);
                    [((l - grow) * sx).max(0.0), ((t - grow) * sy).max(0.0), ((r + grow) * sx).min(w as f32), ((b + grow) * sy).min(h as f32)]
                })
                .collect(),
        ))
    }

    /// The text of one line, and how sure the recogniser is of it (0–1).
    fn read_line(&self, bgr: &dyn Fn(usize, usize) -> [u8; 3], [left, top, right, bottom]: [f32; 4]) -> Result<Option<(Line, f32)>, OcrError> {
        let (bw, bh) = (right - left, bottom - top);
        if bw < 2.0 || bh < 2.0 {
            return Ok(None);
        }
        let width = ((bw / bh * LINE_HEIGHT as f32).ceil() as usize).clamp(LINE_HEIGHT / 2, MAX_LINE_WIDTH);
        let mut input = NdTensor::<f32, 4>::zeros([1, 3, LINE_HEIGHT, width]);
        for y in 0..LINE_HEIGHT {
            let from_y = (top + (y as f32 + 0.5) * bh / LINE_HEIGHT as f32) as usize;
            for x in 0..width {
                let px = bgr((left + (x as f32 + 0.5) * bw / width as f32) as usize, from_y);
                for c in 0..3 {
                    input[[0, c, y, x]] = (f32::from(px[c]) / 255.0 - 0.5) / 0.5;
                }
            }
        }
        let output: NdTensor<f32, 3> =
            self.recognition.run_one(input.view().into(), None).map_err(|e| failed(&e))?.try_into().map_err(|e| failed(&e))?;
        let picks = best_classes(output.view());
        // A speck the detector took for a line (a tone mark above its word, a smudge) reads as
        // a character or two the recogniser is unsure of. PaddleOCR drops those the same way.
        let sure = score(output.view(), &picks);
        if sure < DROP_SCORE {
            return Ok(None);
        }
        let mut words = words_of(&decode(&picks, &self.alphabet), picks.len(), [left, top, right, bottom]);
        for word in &mut words {
            word.rect = ink_rows(bgr, word.rect);
        }
        Ok((!words.is_empty()).then_some((Line { words }, sure)))
    }
}

/// A region this much shorter than the page's median line, or less, may be a mark cut off its line.
const MARK_HEIGHT: f32 = 0.6;
/// How far, in median line heights, a mark may sit from its line.
const MARK_GAP: f32 = 0.6;

/// `regions` with each loose mark joined to its line. The detector can find a Thai tone mark or
/// upper vowel as a region of its own, above the line it belongs to ("น้ำ" then reads "นำ" and
/// the mark becomes a line of its own). A short region whose middle is over or under a taller
/// one, and close to it, is joined to the nearest such line; a short region with no line near
/// stays as it is (a short word, a page number).
fn attach_marks(mut regions: Vec<[f32; 4]>) -> Vec<[f32; 4]> {
    let height = |r: &[f32; 4]| r[3] - r[1];
    let mut heights: Vec<f32> = regions.iter().map(height).collect();
    heights.sort_by(f32::total_cmp);
    let Some(median) = heights.get(heights.len() / 2).copied().filter(|m| *m > 0.0) else { return regions };
    let small = |r: &[f32; 4]| height(r) <= median * MARK_HEIGHT;
    let mut joined = vec![false; regions.len()];
    for i in 0..regions.len() {
        let Some(mark) = regions.get(i).copied().filter(small) else { continue };
        let middle = (mark[0] + mark[2]) / 2.0;
        let gap = |line: &[f32; 4]| {
            if mark[3] <= line[1] {
                line[1] - mark[3]
            } else if mark[1] >= line[3] {
                mark[1] - line[3]
            } else {
                0.0
            }
        };
        let nearest = regions
            .iter()
            .enumerate()
            .filter(|(j, line)| *j != i && !small(line) && line[0] <= middle && middle <= line[2] && gap(line) <= median * MARK_GAP)
            .min_by(|(_, a), (_, b)| gap(a).total_cmp(&gap(b)))
            .map(|(j, _)| j);
        if let Some(line) = nearest.and_then(|j| regions.get_mut(j)) {
            *line = [line[0].min(mark[0]), line[1].min(mark[1]), line[2].max(mark[2]), line[3].max(mark[3])];
            if let Some(done) = joined.get_mut(i) {
                *done = true;
            }
        }
    }
    let mut kept = joined.into_iter();
    regions.retain(|_| !kept.next().unwrap_or(false));
    regions
}

/// Whether boxes `a` and `b` are one piece of text read twice: more than [`COVERED`] of either
/// lies on the other (a short word inside a long line is all on it).
fn same_text(a: &[f32; 4], b: &[f32; 4]) -> bool {
    let area = |r: &[f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let on = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    on > 0.0 && (on > area(a) * COVERED || on > area(b) * COVERED)
}

/// The source pixels `[from, to)` that output pixel `i` of `out` covers, out of `len`: at least
/// one, so a picture made smaller averages every pixel it covers and one made larger repeats them.
fn span(i: usize, len: usize, out: usize) -> (usize, usize) {
    let out = out.max(1);
    let from = (i * len / out).min(len.saturating_sub(1));
    let to = ((i + 1) * len / out).clamp(from + 1, len.max(from + 1));
    (from, to)
}

/// `rect` with its top and bottom on the ink inside it. A region is grown past its text, and the
/// text layer sizes a word by the height of its box. A row dark nearly all the way across is a
/// ruled line, not text. A box with no contrast stays as it is.
fn ink_rows(bgr: &dyn Fn(usize, usize) -> [u8; 3], rect @ [left, top, right, bottom]: [f32; 4]) -> [f32; 4] {
    let (x0, x1, y0, y1) = (left as usize, right.ceil() as usize, top as usize, bottom.ceil() as usize);
    let width = x1.saturating_sub(x0);
    if width == 0 || y1 <= y0 {
        return rect;
    }
    let grey = |x: usize, y: usize| bgr(x, y).iter().map(|c| u32::from(*c)).sum::<u32>() / 3;
    let (mut low, mut high) = (u32::MAX, 0);
    for y in y0..y1 {
        for x in x0..x1 {
            let v = grey(x, y);
            (low, high) = (low.min(v), high.max(v));
        }
    }
    if high < low + 40 {
        return rect;
    }
    let dark = (low + high) / 2;
    let inked = |y: usize| {
        let count = (x0..x1).filter(|x| grey(*x, y) < dark).count();
        count >= 2 && count * 100 < width * 85
    };
    match ((y0..y1).find(|y| inked(*y)), (y0..y1).rev().find(|y| inked(*y))) {
        (Some(first), Some(last)) => [left, first as f32, right, (last + 1) as f32],
        _ => rect,
    }
}

/// The classes the dictionary names, by class number: class 0 is the blank, then the dictionary's
/// characters, then the space.
fn alphabet(dictionary: &str) -> Vec<String> {
    let mut classes = vec![String::new()];
    classes.extend(dictionary.lines().map(|l| l.trim_end_matches('\r').to_owned()));
    classes.push(" ".to_owned());
    classes
}

/// The likeliest class at each time step of a `[1, steps, classes]` output.
fn best_classes(output: NdTensorView<f32, 3>) -> Vec<usize> {
    let [_, steps, classes] = output.shape();
    (0..steps)
        .map(|t| (0..classes).fold((0, f32::NEG_INFINITY), |best, c| if output[[0, t, c]] > best.1 { (c, output[[0, t, c]]) } else { best }).0)
        .collect()
}

/// The mean probability of the characters picked (not the blanks): 0 when there are none.
fn score(output: NdTensorView<f32, 3>, picks: &[usize]) -> f32 {
    let chosen: Vec<f32> = picks.iter().enumerate().filter(|(_, c)| **c != 0).filter_map(|(t, c)| output.get([0, t, *c]).copied()).collect();
    if chosen.is_empty() { 0.0 } else { chosen.iter().sum::<f32>() / chosen.len() as f32 }
}

/// CTC decoding: a run of one class is one character, the blank is none. Each character comes
/// with the time step it starts at.
fn decode(picks: &[usize], alphabet: &[String]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut previous = 0;
    for (t, &class) in picks.iter().enumerate() {
        if class != previous
            && class != 0
            && let Some(text) = alphabet.get(class)
        {
            out.push((t, text.clone()));
        }
        previous = class;
    }
    out
}

/// The characters of a line as words, split at its spaces, each with its share of the line's box
/// by the time steps it spans.
fn words_of(chars: &[(usize, String)], steps: usize, [left, top, right, bottom]: [f32; 4]) -> Vec<Word> {
    let x = |t: usize| left + (right - left) * t as f32 / steps.max(1) as f32;
    let mut words = Vec::new();
    let mut text = String::new();
    let mut start = 0;
    for (i, (t, c)) in chars.iter().enumerate() {
        if c == " " {
            if !text.is_empty() {
                words.push(Word::new(std::mem::take(&mut text), [x(start), top, x(*t), bottom], "paddle"));
            }
            continue;
        }
        if text.is_empty() {
            start = *t;
        }
        text.push_str(c);
        if i + 1 == chars.len() {
            words.push(Word::new(std::mem::take(&mut text), [x(start), top, right, bottom], "paddle"));
        }
    }
    words
}

/// Boxes `[left, top, right, bottom]` of the 4-connected regions of `map` above [`BINARY`] whose
/// mean is at least [`REGION`], a little taller than a speck.
fn components(map: &dyn Fn(usize, usize) -> f32, w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut seen = vec![false; w * h];
    let mut boxes = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        let (sx, sy) = (start % w, start / w);
        if seen[start] || map(sx, sy) < BINARY {
            continue;
        }
        seen[start] = true;
        stack.push((sx, sy));
        let (mut l, mut t, mut r, mut b) = (sx, sy, sx, sy);
        let (mut sum, mut count) = (0.0f32, 0u32);
        while let Some((x, y)) = stack.pop() {
            sum += map(x, y);
            count += 1;
            (l, t, r, b) = (l.min(x), t.min(y), r.max(x), b.max(y));
            let mut visit = |nx: usize, ny: usize| {
                let at = ny * w + nx;
                if !seen[at] && map(nx, ny) >= BINARY {
                    seen[at] = true;
                    stack.push((nx, ny));
                }
            };
            if x > 0 {
                visit(x - 1, y);
            }
            if x + 1 < w {
                visit(x + 1, y);
            }
            if y > 0 {
                visit(x, y - 1);
            }
            if y + 1 < h {
                visit(x, y + 1);
            }
        }
        if count >= 12 && sum / count as f32 >= REGION {
            boxes.push([l as f32, t as f32, r as f32 + 1.0, b as f32 + 1.0]);
        }
    }
    boxes
}

/// Lines top to bottom, those side by side on one row left to right.
fn reading_order(mut lines: Vec<Line>) -> Vec<Line> {
    let rect = |l: &Line| l.words.first().map_or([0.0; 4], |w| w.rect);
    lines.sort_by(|a, b| rect(a)[1].total_cmp(&rect(b)[1]));
    let mut rows: Vec<Vec<Line>> = Vec::new();
    for line in lines {
        let r = rect(&line);
        let middle = (r[1] + r[3]) / 2.0;
        match rows.last_mut().filter(|row| row.first().is_some_and(|f| middle < rect(f)[3])) {
            Some(row) => row.push(line),
            None => rows.push(vec![line]),
        }
    }
    for row in &mut rows {
        row.sort_by(|a, b| rect(a)[0].total_cmp(&rect(b)[0]));
    }
    rows.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dictionary_sits_between_the_blank_and_the_space() {
        let classes = alphabet("\u{0E01}\r\n\u{0E02}\n");
        assert_eq!(classes, ["", "\u{0E01}", "\u{0E02}", " "]);
    }

    /// CTC: repeats are one character, a blank between them makes two, the blank is nothing, and a
    /// class past the alphabet is skipped.
    #[test]
    fn repeats_merge_and_blanks_vanish() {
        let classes = alphabet("a\nb\n");
        let text = |picks: &[usize]| decode(picks, &classes).into_iter().map(|(_, c)| c).collect::<String>();
        assert_eq!(text(&[1, 1, 0, 1, 2, 2, 0, 0, 99, 3, 3, 2]), "aab b");
        assert_eq!(text(&[]), "");
        assert_eq!(text(&[0, 0]), "");
    }

    /// Words share the line's box by the time steps they span, and spaces only separate.
    #[test]
    fn a_line_splits_at_its_spaces() {
        let chars: Vec<(usize, String)> = [(0, "a"), (2, "b"), (5, " "), (6, " "), (7, "c")].map(|(t, c)| (t, c.to_owned())).to_vec();
        let words = words_of(&chars, 10, [0.0, 10.0, 100.0, 30.0]);
        assert_eq!(words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["ab", "c"]);
        assert_eq!(words[0].rect, [0.0, 10.0, 50.0, 30.0]);
        assert_eq!(words[1].rect, [70.0, 10.0, 100.0, 30.0]);
        assert!(words_of(&[(0, " ".to_owned())], 4, [0.0; 4]).is_empty());
        assert!(words_of(&[], 0, [0.0; 4]).is_empty());
    }

    /// Two regions are found apart, a speck and faint pixels are not lines.
    #[test]
    fn regions_are_connected_probable_pixels() {
        let (w, h) = (20, 6);
        let map = |x: usize, y: usize| match (x, y) {
            (1..=6, 1..=2) | (10..=15, 3..=4) => 0.9,
            (18, 5) => 0.9,
            (0, 5) => 0.2,
            _ => 0.0,
        };
        assert_eq!(components(&map, w, h), [[1.0, 1.0, 7.0, 3.0], [10.0, 3.0, 16.0, 5.0]]);
        assert!(components(&|_, _| 0.0, 0, 0).is_empty());
    }

    #[test]
    fn lines_read_top_to_bottom_then_left_to_right() {
        let line = |text: &str, rect: [f32; 4]| Line { words: vec![Word::new(text, rect, "paddle")] };
        let order = reading_order(vec![
            line("below", [0.0, 50.0, 100.0, 70.0]),
            line("right", [200.0, 11.0, 300.0, 31.0]),
            line("left", [0.0, 10.0, 100.0, 30.0]),
        ]);
        assert_eq!(order.iter().map(Line::text).collect::<Vec<_>>(), ["left", "right", "below"]);
    }

    /// A word's box ends at its ink; the ruled line it sits on, and a blank box, don't count.
    #[test]
    fn a_box_shrinks_to_its_ink_and_past_a_ruled_line() {
        let page = |x: usize, y: usize| -> [u8; 3] {
            match (x, y) {
                (_, 18) => [0; 3],
                (10..=30, 6..=12) => [0; 3],
                _ => [255; 3],
            }
        };
        assert_eq!(ink_rows(&page, [0.0, 0.0, 50.0, 25.0]), [0.0, 6.0, 50.0, 13.0]);
        assert_eq!(ink_rows(&|_, _| [255; 3], [0.0, 2.0, 50.0, 25.0]), [0.0, 2.0, 50.0, 25.0]);
        assert_eq!(ink_rows(&page, [5.0, 5.0, 5.0, 5.0]), [5.0, 5.0, 5.0, 5.0]);
    }

    /// A line is as sure as its characters, blanks aside; no characters is no certainty.
    #[test]
    fn a_line_scores_its_characters_only() {
        let output = NdTensor::from_data([1, 3, 2], vec![0.1, 0.9, 1.0, 0.0, 0.4, 0.6]);
        let picks = best_classes(output.view());
        assert_eq!(picks, [1, 0, 1]);
        assert!((score(output.view(), &picks) - 0.75).abs() < 1e-6);
        assert_eq!(score(output.view(), &[0, 0, 0]), 0.0);
        assert!(score(output.view(), &[1, 0, 1]) >= DROP_SCORE && score(output.view(), &[0, 0, 0]) < DROP_SCORE);
    }

    /// A tone mark found apart, just above its line, joins that line; a short word with no line
    /// near, or beside a line rather than over it, stays a region of its own.
    #[test]
    fn a_loose_mark_joins_the_line_under_it() {
        let lines = [[0.0, 100.0, 500.0, 140.0], [0.0, 200.0, 500.0, 240.0], [0.0, 300.0, 500.0, 340.0]];
        let mark = [40.0, 88.0, 52.0, 99.0];
        let mut regions = lines.to_vec();
        regions.push(mark);
        let got = attach_marks(regions);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], [0.0, 88.0, 500.0, 140.0]);
        assert_eq!(got[1..], lines[1..]);
        let far = [40.0, 30.0, 52.0, 41.0];
        let beside = [600.0, 110.0, 640.0, 125.0];
        let mut regions = lines.to_vec();
        regions.extend([far, beside]);
        assert_eq!(attach_marks(regions.clone()), regions);
        assert!(attach_marks(Vec::new()).is_empty());
    }

    /// Two boxes are one text when either lies mostly on the other: a repeat of a line, or a
    /// short word inside a longer line; boxes side by side or barely touching are two.
    #[test]
    fn boxes_mostly_on_each_other_are_one_text() {
        let line = [0.0, 0.0, 100.0, 20.0];
        assert!(same_text(&line, &[10.0, 2.0, 90.0, 18.0]));
        assert!(same_text(&[40.0, 0.0, 60.0, 20.0], &[0.0, 0.0, 300.0, 20.0]), "a word inside a long line");
        assert!(!same_text(&line, &[200.0, 0.0, 240.0, 20.0]));
        assert!(!same_text(&line, &[90.0, 0.0, 130.0, 20.0]), "touching by a quarter of the shorter");
        assert!(!same_text(&line, &[0.0; 4]));
    }

    /// Shrinking averages every pixel covered, growing repeats them, and every span has a pixel.
    #[test]
    fn spans_cover_the_source() {
        assert_eq!((0..2).map(|i| span(i, 4, 2)).collect::<Vec<_>>(), [(0, 2), (2, 4)]);
        assert_eq!((0..4).map(|i| span(i, 2, 4)).collect::<Vec<_>>(), [(0, 1), (0, 1), (1, 2), (1, 2)]);
        assert_eq!(span(0, 1, 0), (0, 1));
    }

    /// A picture that isn't one of the sizes given is refused, not read past its end.
    #[test]
    fn a_short_picture_is_refused_before_the_models() {
        let Some(models) = crate::Models::search_dirs().iter().find_map(|d| ThaiModels::in_dir(d)) else { return };
        let ocr = ThaiOcr::load(&models).unwrap();
        assert!(matches!(ocr.recognize(&[0; 5], 10, 10), Err(OcrError::EmptyImage)));
        assert!(matches!(ocr.recognize(&[], 0, 0), Err(OcrError::EmptyImage)));
    }
}
