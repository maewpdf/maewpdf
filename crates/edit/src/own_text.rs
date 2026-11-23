//! Thai edition: whether a page has text of its own, as opposed to text MaewPDF put there: text
//! boxes typed onto it (added items), the same boxes once merged into the page (Merge text boxes,
//! or Resize pages, which mark the stream `/PCMerged`), and headers, footers and watermarks.
//!
//! Edit recognizes a scanned page's words when it is turned on, but leaves a page with text of its
//! own alone (a flyer with a background picture must not change because Edit was turned on). A
//! scan someone typed a few words onto before it was read is still a scan: reading it adds only the
//! words that aren't already there. Without this it was never read, even after reopening the file.

use crate::added::TAG as ADDED_TAG;
use crate::place::{page_list, stream_tag};
use crate::{Document, EditError, Object};

/// The dictionary key that marks a stream as merged MaewPDF text (`true`). Unlike `/PCMark`, it
/// changes nothing about how the stream is read or edited: the merged text is the page's text.
pub const MERGED_KEY: &[u8] = b"PCMerged";

/// How much of the page's content is read before answering "yes" (has text of its own) to be safe.
const MAX_BYTES: usize = 64 * 1024 * 1024;

/// Whether page `page` (0-based) has text of its own: text drawn by its content other than
/// MaewPDF's (see the module notes), or a form XObject (which may hold text), or an OCR layer.
/// Content that can't be read counts as text, so an unsure answer leaves the page alone.
pub fn page_has_own_text(doc: &Document, page: usize) -> Result<bool, EditError> {
    let p = page_list(doc).into_iter().nth(page).ok_or(EditError::NoSuchPage(page))?;
    let list: Vec<Object> = match p.dict.get(b"Contents").cloned() {
        None => Vec::new(),
        Some(c) => match &*doc.resolve(&c) {
            Object::Array(a) => a.clone(),
            _ => vec![c],
        },
    };
    let xobjects = p
        .dict
        .get(b"Resources")
        .map(|r| doc.resolve(r))
        .and_then(|r| r.as_dict().and_then(|r| r.get(b"XObject").map(|x| doc.resolve(x))))
        .and_then(|x| x.as_dict().cloned())
        .unwrap_or_default();
    let is_form = |name: &[u8]| {
        xobjects.get(name).map(|o| doc.resolve(o)).is_none_or(|o| match &*o {
            Object::Stream(s) => s.dict.name(b"Subtype") != Some(&b"Image"[..]),
            _ => true,
        })
    };
    let mut remaining = MAX_BYTES;
    for o in &list {
        match stream_tag(doc, o).as_deref() {
            // MaewPDF's own: typed boxes and page marks. The q/Q wrapper draws nothing.
            Some(t) if t == ADDED_TAG || matches!(t, "HeaderFooter" | "Watermark" | "Background" | "Wrap") => continue,
            // An OCR layer is the page's text already: it has been read.
            Some("OCR") => return Ok(true),
            _ => {}
        }
        let resolved = doc.resolve(o);
        let Object::Stream(s) = &*resolved else { return Ok(true) };
        if s.dict.get(MERGED_KEY).is_some() {
            continue;
        }
        let Ok(bytes) = s.decoded_within(remaining) else { return Ok(true) };
        remaining = remaining.saturating_sub(bytes.len());
        let parsed = pdfcraft_content::parse(&bytes);
        for op in &parsed.ops {
            match op.op.as_slice() {
                b"Tj" | b"TJ" | b"'" | b"\"" => return Ok(true),
                b"Do" => {
                    let name = op.operands.first().and_then(|n| n.as_name()).unwrap_or_default();
                    if is_form(name) {
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(false)
}
