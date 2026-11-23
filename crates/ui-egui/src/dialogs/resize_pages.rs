//! Thai edition: Resize Pages (Organize pages ▸ Resize pages…, or a page's right-click menu). Pick
//! a paper (A4, Letter…), the size most pages of the document have, or a size of your own, and the
//! pages: their content is scaled evenly to fit, centred, in one undo step (`Edit::ResizePages`).

use egui::{Align, Layout};
use pdfcraft_engine::{Edit, page_resize};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

pub use crate::units::Unit;

/// The size a draft resizes to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// A paper by its short name in [`page_resize::NAMED`].
    Paper(&'static str),
    /// The size most pages of the document have.
    MostPages,
    /// Width × height in points.
    Custom,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResizeDraft {
    pub target: Target,
    /// The custom size, in points as the page is shown.
    pub custom: [f64; 2],
    pub unit: Unit,
    /// Turn the size to each page's orientation (a landscape page becomes A4 landscape).
    pub match_orientation: bool,
    /// 0 all pages, 1 the selection, 2 the range below (1-based, inclusive).
    pub which: u8,
    pub from: usize,
    pub to: usize,
}

impl Default for ResizeDraft {
    fn default() -> Self {
        Self { target: Target::Paper("a4"), custom: [595.28, 841.89], unit: Unit::Millimetres, match_orientation: true, which: 0, from: 1, to: 1 }
    }
}

impl ResizeDraft {
    /// The edit for the draft: `selection` is the selected pages (or the current one), `info` the
    /// document. An error is what to tell the user, untranslated (`notify_tr`).
    pub fn edit(&self, info: &pdfcraft_render::DocInfo, selection: &[usize]) -> Result<Edit, &'static str> {
        let n = info.pages.len();
        let pages: Vec<usize> = match self.which {
            1 => selection.iter().copied().filter(|p| *p < n).collect(),
            2 => (self.from.max(1) - 1..self.to.min(n)).collect(),
            _ => (0..n).collect(),
        };
        if pages.is_empty() {
            return Err("No pages match those choices");
        }
        let (width, height, match_orientation) = match self.target {
            Target::Paper(name) => {
                let (w, h) = page_resize::named(name).ok_or("That paper size isn't known")?;
                (w, h, self.match_orientation)
            }
            // The other pages' size exactly: a landscape page among portrait ones becomes portrait.
            Target::MostPages => {
                let (w, h) = page_resize::most_common(&info.pages).ok_or("No pages match those choices")?;
                (w, h, false)
            }
            Target::Custom => (self.custom[0], self.custom[1], self.match_orientation),
        };
        let range = page_resize::MIN_SIDE..=page_resize::MAX_SIDE;
        if !range.contains(&width) || !range.contains(&height) {
            return Err("The page size must be between 1 point and 200 inches on each side");
        }
        Ok(Edit::ResizePages { pages, width, height, match_orientation })
    }
}

/// The label of a paper for the list: its name in the print settings.
fn paper_label(short: &str) -> &'static str {
    page_resize::NAMED.iter().find(|(s, _)| *s == short).map_or("A4", |(_, full)| *full)
}

/// Draw the dialog body; returns (apply, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens) -> (bool, bool) {
    let Some((i, id)) = app.active_ids() else { return (false, true) };
    let Some(doc) = app.session.get(id) else { return (false, true) };
    let count = doc.info.pages.len().max(1);
    let most = page_resize::most_common(&doc.info.pages);
    let selected = app.views[i].target_pages().len();
    let units = app.units;
    let d = &mut app.resize_draft;
    ui.label(egui::RichText::new(tl!("Resize Pages")).font(theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("The content is scaled evenly to fit the new size, centred. Text stays text.")).color(t.text_muted));
    ui.add_space(8.0);
    egui::Grid::new("resize-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label(tl!("Size"));
        ui.vertical(|ui| {
            let current = match d.target {
                Target::Paper(p) => paper_label(p).to_owned(),
                Target::MostPages => tl!("Same size as most pages").to_owned(),
                Target::Custom => tl!("Custom size").to_owned(),
            };
            craft_ui::ComboBox::from_id_salt("resize-size").selected_text(current).show_ui(ui, |ui| {
                for (short, full) in page_resize::NAMED {
                    ui.selectable_value(&mut d.target, Target::Paper(short), full);
                }
                ui.selectable_value(&mut d.target, Target::MostPages, tl!("Same size as most pages"));
                ui.selectable_value(&mut d.target, Target::Custom, tl!("Custom size"));
            });
            // What the choice is, in the unit chosen in Preferences.
            let shown = match d.target {
                Target::Paper(p) => page_resize::named(p),
                Target::MostPages => most,
                Target::Custom => None,
            };
            if let Some((w, h)) = shown {
                ui.label(egui::RichText::new(units.size(w, h)).color(t.text_muted).small());
            }
        });
        ui.end_row();
        if d.target == Target::Custom {
            let k = d.unit.per_point();
            for (label, idx) in [(tl!("Width"), 0), (tl!("Height"), 1)] {
                let l = ui.label(label);
                let mut v = d.custom[idx] * k;
                let r = ui
                    .add(
                        egui::DragValue::new(&mut v)
                            .speed(d.unit.speed())
                            .range(1.0 * k..=14_400.0 * k)
                            .max_decimals(2)
                            .suffix(format!(" {}", d.unit.label())),
                    )
                    .labelled_by(l.id);
                if r.changed() {
                    d.custom[idx] = (v / k).clamp(page_resize::MIN_SIDE, page_resize::MAX_SIDE);
                }
                ui.end_row();
            }
            ui.label(tl!("Units"));
            craft_ui::ComboBox::from_id_salt("resize-unit").selected_text(d.unit.label()).show_ui(ui, |ui| {
                for u in Unit::ALL {
                    ui.selectable_value(&mut d.unit, u, u.label());
                }
            });
            ui.end_row();
        }
        if d.target != Target::MostPages {
            ui.label("");
            ui.add(widgets::checkbox(&mut d.match_orientation, tl!("Turn the size to each page's orientation")));
            ui.end_row();
        }
        ui.label(tl!("Pages"));
        let mut typing_to = false;
        ui.vertical(|ui| {
            widgets::radio_value(ui, &mut d.which, 0, tl!("All"));
            widgets::radio_value(ui, &mut d.which, 1, crate::i18n::fmt(tl!("Selection ({n})"), &[("n", &selected.to_string())]));
            ui.horizontal(|ui| {
                let from_label = widgets::radio_value(ui, &mut d.which, 2, tl!("From"));
                ui.add_enabled(d.which == 2, egui::DragValue::new(&mut d.from).range(1..=count)).labelled_by(from_label.id);
                let to_label = ui.label(tl!("to"));
                typing_to =
                    widgets::typing_into(&ui.add_enabled(d.which == 2, egui::DragValue::new(&mut d.to).range(1..=count)).labelled_by(to_label.id));
            });
        });
        widgets::keep_range_ordered(d.from, &mut d.to, typing_to);
        ui.end_row();
    });
    ui.add_space(12.0);
    let (mut apply, mut cancel) = (false, false);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if widgets::pill_button(ui, tl!("OK"), true).clicked() {
            widgets::keep_range_ordered(d.from, &mut d.to, false);
            apply = true;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (apply, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(sizes: &[(f64, f64)]) -> (pdfcraft_engine::Session, pdfcraft_engine::DocId) {
        let mut session = pdfcraft_engine::Session::new();
        let (w, h) = sizes[0];
        let blank = session.create_blank(w, h, sizes.len()).unwrap();
        let mut cos = pdfcraft_cos::Document::open(blank).unwrap();
        for (page, (w, h)) in pdfcraft_model::pages(&cos).into_iter().zip(sizes) {
            let media = pdfcraft_cos::Object::Array([0.0, 0.0, *w, *h].map(pdfcraft_cos::Object::Real).to_vec());
            cos.update_dict(page.obj, |d| d.set(b"MediaBox".to_vec(), media)).unwrap();
        }
        let bytes = pdfcraft_cos::write_full(&cos, &pdfcraft_cos::SaveOptions::default()).unwrap();
        let id = session.open_new("sizes.pdf", std::sync::Arc::new(bytes)).unwrap();
        (session, id)
    }

    #[test]
    fn a_large_page_takes_the_size_most_pages_have_in_one_undo_step() {
        let (mut s, id) = session(&[(595.0, 842.0), (1700.0, 2338.0), (595.0, 842.0)]);
        let d = ResizeDraft { target: Target::MostPages, which: 1, ..Default::default() };
        let edit = d.edit(&s.get(id).unwrap().info, &[1]).unwrap();
        s.apply(id, edit).unwrap();
        let doc = s.get(id).unwrap();
        let p = &doc.info.pages[1];
        assert!((p.width - 595.0).abs() < 0.01 && (p.height - 842.0).abs() < 0.01, "{} × {}", p.width, p.height);
        assert_eq!(doc.can_undo(), Some("Resize page"));
        s.undo(id).unwrap();
        let p = &s.get(id).unwrap().info.pages[1];
        assert!((p.width - 1700.0).abs() < 0.01, "undone: {}", p.width);
    }

    #[test]
    fn papers_custom_sizes_and_ranges_make_their_edits() {
        let (s, id) = session(&[(612.0, 792.0), (792.0, 612.0)]);
        let info = &s.get(id).unwrap().info;
        let d = ResizeDraft::default();
        assert_eq!(d.edit(info, &[0]), Ok(Edit::ResizePages { pages: vec![0, 1], width: 595.28, height: 841.89, match_orientation: true }));
        let d =
            ResizeDraft { target: Target::Custom, custom: [300.0, 400.0], match_orientation: false, which: 2, from: 2, to: 2, ..Default::default() };
        assert_eq!(d.edit(info, &[0]), Ok(Edit::ResizePages { pages: vec![1], width: 300.0, height: 400.0, match_orientation: false }));
        let d = ResizeDraft { target: Target::Custom, custom: [0.0, 400.0], ..Default::default() };
        assert!(d.edit(info, &[0]).is_err(), "too small");
        let d = ResizeDraft { which: 1, ..Default::default() };
        assert!(d.edit(info, &[]).is_err(), "no pages");
    }
}
