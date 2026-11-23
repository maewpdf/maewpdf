//! Thai edition: Resize pages (`crate::resize`). The page takes the new size, what it shows is
//! scaled evenly into it, and its text stays text that Edit text reads and rewrites where it now is.

use super::*;
use crate::resize::{MAX_SIDE, PageSize, resize_pages};

fn size(width: f64, height: f64) -> PageSize {
    PageSize { width, height, match_orientation: false }
}

fn media(doc: &Document, page: usize) -> [f64; 4] {
    let p = &pdfcraft_model::pages(doc)[page];
    let v: Vec<f64> = doc.resolve(p.dict.get(b"MediaBox").unwrap()).as_array().unwrap().iter().map(|o| doc.resolve(o).as_f64().unwrap()).collect();
    <[f64; 4]>::try_from(v).unwrap()
}

fn line_rect(doc: &Document, text: &str) -> [f64; 4] {
    text::text_lines(doc, 0).unwrap().into_iter().find(|l| l.text == text).unwrap_or_else(|| panic!("no line {text:?}")).rect
}

fn close(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
}

/// The page's content read as one: `q`/`Q` must balance, or the scale would reach (or miss) what
/// comes after it.
fn depth_at_end(doc: &Document, page: usize) -> i64 {
    let all = streams(doc, page).join("\n");
    let parsed = pdfcraft_content::parse(all.as_bytes());
    parsed.ops.iter().fold(0i64, |d, op| match op.op.as_slice() {
        b"q" => d + 1,
        b"Q" => d - 1,
        _ => d,
    })
}

#[test]
fn a_page_is_resized_and_its_text_scales_with_it() {
    let mut doc = mixed_region_page();
    let about = line_rect(&doc, "About");
    resize_pages(&mut doc, &[0], size(300.0, 400.0)).unwrap();
    let doc = reopen(&doc);
    assert_eq!(media(&doc, 0), [0.0, 0.0, 300.0, 400.0]);
    // Half the size, from the same corner (the page and the new size have one shape).
    assert!(close(line_rect(&doc, "About"), about.map(|v| v * 0.5)), "{:?} vs {about:?}", line_rect(&doc, "About"));
    assert_eq!(depth_at_end(&doc, 0), 0);
    // The paragraphs are found as before.
    let blocks = text::text_blocks(&doc, 0).unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[1].editor_text, "About\nPlain bold text.\nItalic footer");
}

/// What the owner cares most about: after a resize, Edit text still rewrites a paragraph with
/// several styles in place, each style kept, where the page now shows it.
#[test]
fn a_mixed_style_paragraph_is_still_edited_in_place_after_a_resize() {
    let mut doc = mixed_region_page();
    let before = text::text_lines(&doc, 0).unwrap();
    resize_pages(&mut doc, &[0], size(300.0, 400.0)).unwrap();
    let block = text::text_blocks(&doc, 0).unwrap()[1].clone();
    let draft = block.editor_text.replace("Plain", "Clear");
    text::replace_block(&mut doc, 0, 1, &draft).unwrap();
    let doc = reopen(&doc);
    let lines = text::text_lines(&doc, 0).unwrap();
    for word in ["About", "Italic footer"] {
        let old = before.iter().find(|l| l.text == word).unwrap();
        let new = lines.iter().find(|l| l.text == word).unwrap_or_else(|| panic!("{word} is kept: {lines:?}"));
        assert_eq!(new.font, old.font, "{word} keeps its font");
        assert_eq!(new.color, old.color, "{word} keeps its colour");
        assert!(close(new.rect, old.rect.map(|v| v * 0.5)), "{word} is where the resized page shows it: {:?}", new.rect);
    }
    let edited = text::text_blocks(&doc, 0).unwrap();
    assert_eq!(edited[1].editor_text, "About\nClear bold text.\nItalic footer");
    // The bold word keeps its own font and colour inside the edited line.
    let bold = lines.iter().find(|l| l.text.contains("bold")).unwrap();
    assert!(bold.text.contains("bold"), "{lines:?}");
}

/// Thai in a styled range, written after the resize, is shaped in the chosen face and lands inside
/// the resized page.
#[test]
fn thai_in_a_styled_range_is_written_on_a_resized_page() {
    if crate::added::Family::Sarabun.craft(false, true).is_none() {
        return;
    }
    let mut doc = mixed_region_page();
    resize_pages(&mut doc, &[0], size(300.0, 400.0)).unwrap();
    let block = text::text_blocks(&doc, 0).unwrap()[1].clone();
    let draft = format!("{} สวัสดี", block.editor_text);
    let start = draft.find("สวัสดี").unwrap();
    let style = BlockStyle {
        ranges: vec![RangeStyle {
            range: start..draft.len(),
            family: Some((Family::Sarabun, false, true)),
            resource: None,
            size: None,
            color: None,
            underline: None,
            char_spacing: None,
            scale: None,
        }],
        ..Default::default()
    };
    text::rewrite_block(&mut doc, 0, 1, Some(&draft), &style).unwrap();
    let doc = reopen(&doc);
    let lines = text::text_lines(&doc, 0).unwrap();
    let thai = lines.iter().find(|l| l.text.contains("สวัสดี")).unwrap_or_else(|| panic!("the Thai is one run: {lines:?}"));
    assert!(thai.italic, "in the italic face: {}", thai.base_font);
    assert!(thai.rect[2] <= 300.0 && thai.rect[3] <= 400.0, "on the resized page: {:?}", thai.rect);
}

#[test]
fn a_page_of_another_shape_is_centred_in_the_new_size() {
    let mut doc = mixed_region_page();
    let about = line_rect(&doc, "About");
    // 600 × 800 into a square: scaled by 0.5 to 300 × 400, centred across.
    resize_pages(&mut doc, &[0], size(400.0, 400.0)).unwrap();
    let want = [about[0] * 0.5 + 50.0, about[1] * 0.5, about[2] * 0.5 + 50.0, about[3] * 0.5];
    assert!(close(line_rect(&doc, "About"), want), "{:?} vs {want:?}", line_rect(&doc, "About"));
}

#[test]
fn a_landscape_page_takes_the_landscape_size_when_orientation_is_matched() {
    // Page 2 of the fixture is turned 90°: 600 × 800 unturned, 800 × 600 as shown.
    let mut doc = fixture();
    resize_pages(&mut doc, &[1], PageSize { width: 595.0, height: 842.0, match_orientation: true }).unwrap();
    let p = pdfcraft_model::pages(&doc)[1].clone();
    let (w, h) = p.display_size(&doc);
    assert!((w - 842.0).abs() < 0.01 && (h - 595.0).abs() < 0.01, "shown landscape: {w} × {h}");
    assert_eq!(p.rotation(&doc), 90, "the page keeps its turn");
    // Without matching, the size is taken as given.
    let mut doc = fixture();
    resize_pages(&mut doc, &[1], size(595.0, 842.0)).unwrap();
    let (w, h) = pdfcraft_model::pages(&doc)[1].display_size(&doc);
    assert!((w - 595.0).abs() < 0.01 && (h - 842.0).abs() < 0.01, "{w} × {h}");
}

#[test]
fn the_user_unit_is_folded_into_the_new_size() {
    let mut doc = mixed_region_page();
    let page = pdfcraft_model::pages(&doc)[0].clone();
    doc.update_dict(page.obj, |d| d.set(b"UserUnit".to_vec(), Object::Real(2.0))).unwrap();
    // 600 × 800 units of 2 pt is 1200 × 1600 pt; A quarter of that is 300 × 400 pt.
    resize_pages(&mut doc, &[0], size(300.0, 400.0)).unwrap();
    let p = pdfcraft_model::pages(&doc)[0].clone();
    assert!(p.dict.get(b"UserUnit").is_none());
    assert_eq!(p.display_size(&doc), (300.0, 400.0));
    assert!(close(line_rect(&doc, "About"), line_rect(&mixed_region_page(), "About").map(|v| v * 0.5)));
}

#[test]
fn annotations_move_with_the_page() {
    let mut doc = mixed_region_page();
    let annot = doc.add(Object::Dict({
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("Annot"));
        d.set(b"Subtype".to_vec(), Object::name("Highlight"));
        d.set(b"Rect".to_vec(), Object::Array([72.0, 598.0, 110.0, 612.0].map(Object::Real).to_vec()));
        d.set(b"QuadPoints".to_vec(), Object::Array([72.0, 612.0, 110.0, 612.0, 72.0, 598.0, 110.0, 598.0].map(Object::Real).to_vec()));
        d
    }));
    let page = pdfcraft_model::pages(&doc)[0].clone();
    doc.update_dict(page.obj, |d| d.set(b"Annots".to_vec(), Object::Array(vec![Object::Ref(annot)]))).unwrap();
    resize_pages(&mut doc, &[0], size(300.0, 400.0)).unwrap();
    let d = doc.get(annot).as_dict().cloned().unwrap();
    let nums = |k: &[u8]| -> Vec<f64> { d.get(k).unwrap().as_array().unwrap().iter().map(|o| o.as_f64().unwrap()).collect() };
    assert_eq!(nums(b"Rect"), [36.0, 299.0, 55.0, 306.0]);
    assert_eq!(nums(b"QuadPoints"), [36.0, 306.0, 55.0, 306.0, 36.0, 299.0, 55.0, 299.0]);
}

#[test]
fn marks_and_tagged_layers_keep_their_tags_and_balance() {
    let mut doc = mixed_region_page();
    let mut hf = HeaderFooter::default();
    hf.text[1] = "Header".into();
    add_header_footer(&mut doc, &[0], &hf, false, &cx()).unwrap();
    stamp(&mut doc, 0, "OCR", b"BT /PCHelv 10 Tf 3 Tr 72 700 Td (scan) Tj ET".to_vec()).unwrap();
    let tags_before: Vec<Option<String>> = {
        let p = pdfcraft_model::pages(&doc)[0].clone();
        let list = crate::place::contents(&mut doc.clone(), &p).unwrap();
        list.iter().map(|o| crate::place::stream_tag(&doc, o)).collect()
    };
    resize_pages(&mut doc, &[0], size(300.0, 400.0)).unwrap();
    let doc = reopen(&doc);
    let p = pdfcraft_model::pages(&doc)[0].clone();
    let list = crate::place::contents(&mut doc.clone(), &p).unwrap();
    let tags: Vec<Option<String>> = list.iter().map(|o| crate::place::stream_tag(&doc, o)).collect();
    // Each tagged layer is still there, tagged as before, in the same order.
    let named = |t: &[Option<String>]| t.iter().flatten().cloned().collect::<Vec<_>>();
    assert_eq!(named(&tags), named(&tags_before));
    assert_eq!(depth_at_end(&doc, 0), 0);
    // The header and the OCR layer are scaled with the page: half the size, half the place.
    let lines = text::text_lines(&doc, 0).unwrap();
    let header = lines.iter().find(|l| l.text == "Header").unwrap();
    assert!(header.rect[3] <= 400.0 && header.rect[1] > 300.0, "{:?}", header.rect);
    let scan = lines.iter().find(|l| l.text == "scan").unwrap();
    assert!((scan.rect[0] - 36.0).abs() < 0.05, "{:?}", scan.rect);
    // A mark added afterwards is drawn in the new page's space, not scaled again.
    let mut doc = doc;
    let mut hf = HeaderFooter::default();
    hf.text[4] = "Footer".into();
    add_header_footer(&mut doc, &[0], &hf, false, &cx()).unwrap();
    let footer = text::text_lines(&doc, 0).unwrap().into_iter().find(|l| l.text == "Footer").unwrap();
    // Its baseline 36 pt above the bottom of the new page (the box reaches below it by the descent).
    assert!(footer.rect[1] > 30.0 && footer.rect[1] < 40.0, "{:?}", footer.rect);
    assert_eq!(depth_at_end(&doc, 0), 0);
}

#[test]
fn a_stray_q_cannot_end_the_scale_early() {
    let mut doc = text_page("Q BT /F1 10 Tf 72 600 Td (After) Tj ET");
    resize_pages(&mut doc, &[0], size(300.0, 400.0)).unwrap();
    let r = line_rect(&doc, "After");
    // Scaled with the rest: its baseline at 300 (the box reaches below it by the descent).
    assert!((r[0] - 36.0).abs() < 0.05 && r[1] > 297.0 && r[1] < 300.0, "{r:?}");
}

#[test]
fn impossible_sizes_and_pages_are_refused_unchanged() {
    let mut doc = mixed_region_page();
    let original = streams(&doc, 0);
    for bad in [0.0, -5.0, f64::NAN, f64::INFINITY, MAX_SIDE + 1.0] {
        assert!(resize_pages(&mut doc, &[0], size(bad, 400.0)).is_err(), "{bad}");
        assert!(resize_pages(&mut doc, &[0], size(300.0, bad)).is_err(), "{bad}");
    }
    assert!(matches!(resize_pages(&mut doc, &[3], size(300.0, 400.0)), Err(EditError::NoSuchPage(3))));
    assert_eq!(streams(&doc, 0), original);
    assert_eq!(media(&doc, 0), [0.0, 0.0, 600.0, 800.0]);
}

/// A page with MaewPDF text boxes is resized too: the boxes become part of the page (as Merge text
/// boxes makes them), scaled with the rest, and their text is the page's to edit.
#[test]
fn text_boxes_become_part_of_the_resized_page() {
    let mut doc = mixed_region_page();
    add_content(&mut doc, 0, &Content::Text(AddedText { rect: [72.0, 300.0, 300.0, 340.0], text: "Box".into(), ..AddedText::default() })).unwrap();
    let before = line_rect(&doc, "Box");
    resize_pages(&mut doc, &[0], size(300.0, 400.0)).unwrap();
    let doc = reopen(&doc);
    assert!(list_added(&doc).is_empty(), "no added items are left to draw from the old model");
    assert!(close(line_rect(&doc, "Box"), before.map(|v| v * 0.5)), "{:?} vs {before:?}", line_rect(&doc, "Box"));
    assert!(text::text_blocks(&doc, 0).unwrap().iter().any(|b| b.editor_text.contains("Box")), "the box's text is the page's text");
    assert_eq!(depth_at_end(&doc, 0), 0);
}
