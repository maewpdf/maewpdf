//! Thai edition: the editing tips (Ctrl+E Edit text & images, Ctrl+T Add text) come up once with
//! the first document, and again from Help › Editing tips.
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

fn harness(offer: bool, saved: &str) -> Harness<'static, PdfCraftApp> {
    let saved = saved.to_owned();
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.restore(&saved);
        app.set_option("language", "en").unwrap();
        app.offer_editing_tips = offer;
        app
    })
}

/// An empty one-page PDF.
fn blank() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << >> >>",
    ];
    let mut pdf = b"%PDF-1.7
"
    .to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(
            format!(
                "{} 0 obj
{o}
endobj
",
                i + 1
            )
            .as_bytes(),
        );
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!(
            "xref
0 {}
0000000000 65535 f 
",
            objects.len() + 1
        )
        .as_bytes(),
    );
    for o in offsets {
        pdf.extend_from_slice(
            format!(
                "{o:010} 00000 n 
"
            )
            .as_bytes(),
        );
    }
    pdf.extend_from_slice(
        format!(
            "trailer
<< /Size {} /Root 1 0 R >>
startxref
{xref}
%%EOF
",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn open(h: &mut Harness<'static, PdfCraftApp>) {
    h.state_mut().open_bytes("tips.pdf", None, blank()).unwrap();
    crate::support::settle(h);
}

#[test]
fn the_first_document_brings_the_tips_once_with_the_real_keys() {
    let mut h = harness(true, "{}");
    crate::support::settle(&mut h);
    assert_eq!(h.state().dialog, None, "nothing before a document opens");
    open(&mut h);
    assert_eq!(h.state().dialog, Some(Dialog::EditingTips));
    // The keys come from the command registry (kittest's context isn't a Mac).
    h.get_by_label("Ctrl+E");
    h.get_by_label("Ctrl+T");
    h.get_by_label("Edit text & images");
    h.get_by_label("Add text");
    h.get_by_label("Got it").click();
    crate::support::settle(&mut h);
    assert_eq!(h.state().dialog, None);
    // Remembered: a second document, or the next launch, doesn't bring them back.
    open(&mut h);
    assert_eq!(h.state().dialog, None);
    let saved = h.state().persist();
    let mut next = harness(true, &saved);
    open(&mut next);
    assert_eq!(next.state().dialog, None, "shown once across launches");
}

#[test]
fn tests_and_embedders_without_the_offer_never_see_them_and_help_opens_them() {
    let mut h = harness(false, "{}");
    open(&mut h);
    assert_eq!(h.state().dialog, None);
    assert!(h.state_mut().execute("help.editing_tips"));
    crate::support::settle(&mut h);
    assert_eq!(h.state().dialog, Some(Dialog::EditingTips));
    h.get_by_label("Got it");
}

/// The tips come in four tabs: editing text first, then layout (rulers and guides), scans and OCR
/// (cleaning a scan up), then general (the theme, zooming, moving around).
#[test]
fn the_tips_tabs_show_layout_and_general_tips() {
    let mut h = harness(false, "{}");
    open(&mut h);
    assert!(h.state_mut().execute("help.editing_tips"));
    crate::support::settle(&mut h);
    h.get_by_label("Nudge");
    assert!(h.query_by_label("Guides").is_none(), "one tab at a time");
    // The accessibility click: kittest's pointer click on these tabs is not seen as a click
    // (it is in the app, checked through the control channel).
    h.get_by_label("Layout").click_accesskit();
    crate::support::settle(&mut h);
    h.get_by_label("Rulers");
    h.get_by_label("Guides");
    assert!(h.query_by_label("Nudge").is_none());
    h.get_by_label("Scans & OCR").click_accesskit();
    crate::support::settle(&mut h);
    h.get_by_label("Enhance scanned file");
    h.get_by_label("Shadows and specks");
    h.get_by_label("General").click_accesskit();
    crate::support::settle(&mut h);
    h.get_by_label("Light or dark");
    h.get_by_label("Move around the page");
    h.get_by_label("Got it").click();
    crate::support::settle(&mut h);
    assert_eq!(h.state().dialog, None);
}
