//! Resize pages (Thai edition): give pages a new paper size (A4, Letter, the size of the other
//! pages…) and scale what they show to fit it, evenly and centred, as a scan much larger than the
//! rest of its file needs. Nothing is rasterised: the page's content stays as it is, drawn through
//! one `cm`, so text stays text (an OCR layer stays searchable and editable) and pictures keep
//! their pixels.
//!
//! How it is stored, so the page's layers (Arrange) and marks keep working: each layer of the page
//! gets the scale inside it. The page's own content (a PdfCraft `q`/`Q` wrapper and what it holds,
//! or a run of untagged streams) gets two small untagged streams, `q <k> 0 0 <k> <x> <y> cm` after
//! its start and `Q` before its end; a tagged layer drawn on its own (a mark, an OCR text layer, a
//! lifted paragraph, an edit drawn on top) gets the same `q … cm` and `Q` written into its stream,
//! keeping its tag. Edit text reads the `cm` like any page's own. Content added after the resize (a
//! mark on top, a text box, an OCR layer) is drawn in the new page's space. The page's boxes and
//! its annotations are moved the same way; an appearance follows its annotation's `/Rect`.
//!
//! A page with MaewPDF text boxes (added items) is refused: they are drawn from the stored model
//! in page space, which the scale would not reach. Merging them into the page's text first (or
//! deleting them) makes the page resizable.

use std::collections::HashSet;

use crate::added::TAG as ADDED_TAG;
use crate::place::{check, contents, page_list, stream_tag, wrapper_role};
use crate::{Document, EditError, ObjRef, Object, Stream};

/// The smallest and largest side a page may be resized to, in points: Acrobat's page-size range
/// (1 pt to 200 in).
pub const MIN_SIDE: f64 = 1.0;
pub const MAX_SIDE: f64 = 14_400.0;

/// The most content streams or bytes read to balance a page's `q`/`Q`. Past them the page is
/// refused rather than resized without knowing where its content ends.
const BALANCE_MAX_BYTES: usize = 64 * 1024 * 1024;
const BALANCE_MAX_STREAMS: usize = 100_000;

/// How far a page's content is moved: `x' = k·x + tx`, `y' = k·y + ty`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    k: f64,
    tx: f64,
    ty: f64,
}

impl Fit {
    fn point(&self, x: f64, y: f64) -> (f64, f64) {
        (self.k * x + self.tx, self.k * y + self.ty)
    }

    fn rect(&self, r: [f64; 4]) -> [f64; 4] {
        let (x0, y0) = self.point(r[0], r[1]);
        let (x1, y1) = self.point(r[2], r[3]);
        [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
    }
}

/// The new size of a page, in points as it is displayed (after `/Rotate`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub width: f64,
    pub height: f64,
    /// Turn the size to each page's own orientation: a landscape page resized to A4 becomes A4
    /// landscape. Off, every page gets `width` × `height` as given.
    pub match_orientation: bool,
}

/// Resize `pages` (0-based) to `size`, scaling each page's content evenly to fit, centred. Every
/// page is checked before any changes, so an error leaves the document as it was.
pub fn resize_pages(doc: &mut Document, pages: &[usize], size: PageSize) -> Result<(), EditError> {
    let PageSize { width, height, match_orientation } = size;
    let in_range = |v: f64| v.is_finite() && (MIN_SIDE..=MAX_SIDE).contains(&v);
    if !in_range(width) || !in_range(height) {
        return Err(EditError::Invalid(format!("the page size must be between {MIN_SIDE} and {MAX_SIDE} points on each side")));
    }
    let all = page_list(doc);
    check(pages, all.len())?;
    let mut seen_pages = HashSet::new();
    let mut plan = Vec::new();
    for &i in pages {
        if !seen_pages.insert(i) {
            continue;
        }
        let Some(page) = all.get(i) else { return Err(EditError::NoSuchPage(i)) };
        let list = contents(doc, page)?;
        if list.iter().any(|o| stream_tag(doc, o).as_deref() == Some(ADDED_TAG)) {
            return Err(EditError::Invalid(format!(
                "page {} has text boxes added in MaewPDF; merge them into the page's text or delete them, then resize the page",
                i + 1
            )));
        }
        let crop = page.crop(doc);
        let unit = page.user_unit(doc);
        let turned = page.rotation(doc) % 180 != 0;
        // The size as the page is shown, turned to its orientation when asked.
        let (shown_w, shown_h) = page.display_size(doc);
        let (mut want_w, mut want_h) = (width, height);
        if match_orientation && (shown_w > shown_h) != (want_w > want_h) && want_w != want_h {
            std::mem::swap(&mut want_w, &mut want_h);
        }
        // The new media box is in the unturned page's space.
        let (box_w, box_h) = if turned { (want_h, want_w) } else { (want_w, want_h) };
        let (cw, ch) = ((crop[2] - crop[0]) * unit, (crop[3] - crop[1]) * unit);
        if !(cw.is_finite() && ch.is_finite() && cw > 0.0 && ch > 0.0) {
            return Err(EditError::Invalid(format!("page {} has no area to resize", i + 1)));
        }
        let s = (box_w / cw).min(box_h / ch);
        // Content is in user-space units of `unit` points; the resized page counts in points.
        let k = s * unit;
        let fit = Fit { k, tx: (box_w - cw * s) / 2.0 - crop[0] * k, ty: (box_h - ch * s) / 2.0 - crop[1] * k };
        if ![fit.k, fit.tx, fit.ty].iter().all(|v| v.is_finite()) || fit.k <= 0.0 {
            return Err(EditError::Invalid(format!("page {} can't be scaled to that size", i + 1)));
        }
        plan.push((page.clone(), list, fit, [0.0, 0.0, box_w, box_h]));
    }
    // Every page's new content is worked out before any page changes.
    let mut rewritten = Vec::with_capacity(plan.len());
    for (page, list, fit, media) in plan {
        let new_list = scaled_layers(doc, &list, fit).map_err(|e| EditError::Invalid(format!("page {}: {e}", page_number(doc, page.obj))))?;
        rewritten.push((page, new_list, fit, media));
    }
    let mut moved_annots = HashSet::new();
    for (page, new_list, fit, media) in rewritten {
        let new_list: Vec<Object> = new_list
            .into_iter()
            .map(|entry| match entry {
                Entry::Keep(o) => o,
                Entry::New(s) => Object::Ref(doc.add(Object::Stream(s))),
            })
            .collect();
        let page_dict = doc.get(page.obj).as_dict().cloned().unwrap_or_default();
        let old_boxes: Vec<(&[u8], Option<[f64; 4]>)> =
            [&b"BleedBox"[..], b"TrimBox", b"ArtBox"].into_iter().map(|k| (k, rect_of(doc, page.dict.get(k)))).collect();
        let annots = page_dict.get(b"Annots").cloned();
        doc.update_dict(page.obj, |d| {
            d.set(b"Contents".to_vec(), Object::Array(new_list));
            d.set(b"MediaBox".to_vec(), real_array(&media));
            d.remove(b"CropBox");
            d.remove(b"UserUnit");
            for (key, old) in &old_boxes {
                match old.map(|r| clip(fit.rect(r), media)) {
                    Some(Some(r)) => d.set(key.to_vec(), real_array(&r)),
                    _ => {
                        d.remove(key);
                    }
                }
            }
        })?;
        if let Some(annots) = annots {
            move_annotations(doc, page.obj, annots, fit, &mut moved_annots)?;
        }
    }
    Ok(())
}

/// 1-based number of the page `obj`, for messages.
fn page_number(doc: &Document, obj: ObjRef) -> usize {
    page_list(doc).iter().position(|p| p.obj == obj).map_or(0, |i| i + 1)
}

/// One entry of a page's new `/Contents`: an existing one, or a new stream to add.
enum Entry {
    Keep(Object),
    New(Stream),
}

/// The page's content list with `fit` inside each layer (see the module notes).
fn scaled_layers(doc: &Document, list: &[Object], fit: Fit) -> Result<Vec<Entry>, String> {
    let cm = format!("q\n{} 0 0 {} {} {} cm\n", num(fit.k), num(fit.k), num(fit.tx), num(fit.ty));
    let open = |stray: usize| {
        let mut b = cm.clone().into_bytes();
        // A stray `Q` in the layer pops one of these instead of the scale.
        b.extend_from_slice(&b"q\n".repeat(stray));
        Entry::New(Stream::from_raw(crate::Dict::new(), b))
    };
    let close = |n: usize| Entry::New(Stream::from_raw(crate::Dict::new(), b"\nQ".repeat(n)));
    let mut out = Vec::with_capacity(list.len().saturating_add(4));
    let mut i = 0;
    while let Some(o) = list.get(i) {
        if wrapper_role(doc, o) == Some(true) {
            // A PdfCraft wrapper: the scale goes just inside it, and its close (which already
            // closes whatever its content leaves open) ends the scale too.
            let mut depth = 0usize;
            let mut end = None;
            for (j, w) in list.iter().enumerate().skip(i) {
                match wrapper_role(doc, w) {
                    Some(true) => depth = depth.saturating_add(1),
                    Some(false) => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            end = Some(j);
                            break;
                        }
                    }
                    None => {}
                }
            }
            if let Some(end) = end {
                let inner = list.get(i + 1..end).unwrap_or_default();
                let (stray, _) = balance(doc, inner)?;
                out.push(Entry::Keep(o.clone()));
                out.push(open(stray));
                out.extend(inner.iter().cloned().map(Entry::Keep));
                // With nothing left open this pops the scale's `q`; otherwise it pops one the
                // content left open, and the wrapper's close pops the rest and the scale's.
                out.push(close(1));
                if let Some(c) = list.get(end) {
                    out.push(Entry::Keep(c.clone()));
                }
                i = end + 1;
                continue;
            }
        }
        let tag = stream_tag(doc, o);
        if tag.is_none() || tag.as_deref() == Some("Wrap") {
            // A run of the page's own (untagged) content, or a wrapper part without its pair.
            let start = i;
            while list.get(i).is_some_and(|o| {
                let t = stream_tag(doc, o);
                (t.is_none() || t.as_deref() == Some("Wrap")) && !(i > start && wrapper_role(doc, o) == Some(true))
            }) {
                i += 1;
            }
            let run = list.get(start..i).unwrap_or_default();
            let (stray, open_at_end) = balance(doc, run)?;
            out.push(open(stray));
            out.extend(run.iter().cloned().map(Entry::Keep));
            out.push(close(open_at_end.saturating_add(1)));
            continue;
        }
        // A tagged layer drawn on its own: the scale goes into its stream, which keeps its tag.
        let resolved = doc.resolve(o);
        let Object::Stream(s) = &*resolved else { return Err("its content isn't a stream".into()) };
        let bytes = s.decoded_within(BALANCE_MAX_BYTES).map_err(|_| "its content can't be read".to_owned())?;
        let (stray, open_at_end) = balance_bytes(std::slice::from_ref(&bytes));
        let mut data = cm.clone().into_bytes();
        data.extend_from_slice(&b"q\n".repeat(stray));
        data.extend_from_slice(&bytes);
        data.extend_from_slice(&b"\nQ".repeat(open_at_end.saturating_add(1)));
        let mut dict = s.dict.clone();
        for key in [&b"Filter"[..], b"DecodeParms", b"Length", b"DL", b"F", b"FFilter", b"FDecodeParms"] {
            dict.remove(key);
        }
        out.push(Entry::New(Stream::flate(dict, &data)));
        i += 1;
    }
    Ok(out)
}

/// How many stray `Q` (a `Q` with no `q` open) the content of `list` has, and how many `q` it
/// leaves open at its end.
fn balance(doc: &Document, list: &[Object]) -> Result<(usize, usize), String> {
    if list.len() > BALANCE_MAX_STREAMS {
        return Err("it has too many content streams".into());
    }
    let mut remaining = BALANCE_MAX_BYTES;
    let mut data = Vec::with_capacity(list.len());
    for o in list {
        let resolved = doc.resolve(o);
        let Object::Stream(s) = &*resolved else { return Err("its content isn't a stream".into()) };
        let bytes = s.decoded_within(remaining).map_err(|_| "its content can't be read".to_owned())?;
        remaining = remaining.checked_sub(bytes.len().saturating_add(1)).ok_or_else(|| "its content is too large".to_owned())?;
        data.push(bytes);
    }
    Ok(balance_bytes(&data))
}

fn balance_bytes(data: &[Vec<u8>]) -> (usize, usize) {
    let pieces = pdfcraft_content::Pieces::join(data);
    let parsed = pieces.parse();
    let (mut depth, mut stray) = (0usize, 0usize);
    for op in &parsed.ops {
        match op.op.as_slice() {
            b"q" => depth = depth.saturating_add(1),
            b"Q" if depth == 0 => stray = stray.saturating_add(1),
            b"Q" => depth -= 1,
            _ => {}
        }
    }
    (stray, depth)
}

/// Move every annotation of the page (`/Annots`, a reference or an array) by `fit`. One shared by
/// several resized pages moves once.
fn move_annotations(doc: &mut Document, page: ObjRef, annots: Object, fit: Fit, moved: &mut HashSet<ObjRef>) -> Result<(), EditError> {
    let (list, holder) = match annots {
        Object::Ref(r) => (doc.get(r).as_array().cloned().unwrap_or_default(), Some(r)),
        Object::Array(a) => (a, None),
        _ => return Ok(()),
    };
    let mut changed_inline = false;
    let mut new_list = Vec::with_capacity(list.len());
    for a in list {
        match a {
            Object::Ref(r) => {
                if moved.insert(r)
                    && let Some(d) = doc.get(r).as_dict().cloned()
                {
                    let d = moved_annotation(doc, d, fit);
                    doc.set(r, Object::Dict(d));
                }
                new_list.push(Object::Ref(r));
            }
            Object::Dict(d) => {
                new_list.push(Object::Dict(moved_annotation(doc, d, fit)));
                changed_inline = true;
            }
            other => new_list.push(other),
        }
    }
    if changed_inline {
        match holder {
            Some(r) => doc.set(r, Object::Array(new_list)),
            None => doc.update_dict(page, |d| d.set(b"Annots".to_vec(), Object::Array(new_list)))?,
        }
    }
    Ok(())
}

/// An annotation's geometry moved by `fit` (ISO 32000-2 §12.5): `/Rect`, the point lists
/// (`/QuadPoints`, `/InkList`, `/Vertices`, `/L`, `/CL`) and the `/RD` insets. Entries that
/// aren't numbers are left as they are.
fn moved_annotation(doc: &Document, mut d: crate::Dict, fit: Fit) -> crate::Dict {
    if let Some(r) = rect_of(doc, d.get(b"Rect")) {
        d.set(b"Rect".to_vec(), real_array(&fit.rect(r)));
    }
    for key in [&b"QuadPoints"[..], b"Vertices", b"L", b"CL"] {
        if let Some(points) = d.get(key).and_then(|o| numbers(doc, o)) {
            d.set(key.to_vec(), real_array(&moved_points(&points, fit)));
        }
    }
    if let Some(Object::Array(strokes)) = d.get(b"InkList").map(|o| (*doc.resolve(o)).clone()) {
        let moved: Vec<Object> = strokes.iter().map(|s| numbers(doc, s).map_or_else(|| s.clone(), |p| real_array(&moved_points(&p, fit)))).collect();
        d.set(b"InkList".to_vec(), Object::Array(moved));
    }
    if let Some(rd) = d.get(b"RD").and_then(|o| numbers(doc, o)).filter(|v| v.len() == 4) {
        d.set(b"RD".to_vec(), real_array(&rd.iter().map(|v| v * fit.k).collect::<Vec<_>>()));
    }
    d
}

fn moved_points(points: &[f64], fit: Fit) -> Vec<f64> {
    points
        .chunks(2)
        .flat_map(|p| match p {
            [x, y] => {
                let (x, y) = fit.point(*x, *y);
                vec![x, y]
            }
            other => other.to_vec(),
        })
        .collect()
}

/// An array of numbers (resolving references), or `None` when any entry isn't a finite number.
fn numbers(doc: &Document, o: &Object) -> Option<Vec<f64>> {
    let resolved = doc.resolve(o);
    let a = resolved.as_array()?;
    a.iter().map(|v| doc.resolve(v).as_f64().filter(|x| x.is_finite())).collect()
}

fn rect_of(doc: &Document, o: Option<&Object>) -> Option<[f64; 4]> {
    let v = numbers(doc, o?)?;
    let [x0, y0, x1, y1] = <[f64; 4]>::try_from(v).ok()?;
    Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
}

fn clip(r: [f64; 4], to: [f64; 4]) -> Option<[f64; 4]> {
    let c = [r[0].max(to[0]), r[1].max(to[1]), r[2].min(to[2]), r[3].min(to[3])];
    (c[2] > c[0] && c[3] > c[1]).then_some(c)
}

fn real_array(v: &[f64]) -> Object {
    Object::Array(v.iter().map(|x| Object::Real(*x)).collect())
}

/// A number for a content stream: plain decimal, no exponent, trailing zeros trimmed.
fn num(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".to_owned() } else { s.to_owned() }
}
