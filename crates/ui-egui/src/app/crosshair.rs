//! Thai edition: the crosshair the page tools show (measure, shapes, crop, links…) drawn by
//! MaewPDF, black with a white edge. The system's crosshair follows the Windows cursor scheme;
//! Windows 11's default one is white with a thin black edge (`C:\Windows\Cursors\cross.svg`), and
//! on white paper it can hardly be seen (a user's report, 2026-11-24). Black with a white edge
//! shows on paper and on dark pages alike.
//!
//! The page modules still claim `CursorIcon::Crosshair` (`page_cursor`); once the frame is drawn,
//! a crosshair the app is about to show is hidden and this one drawn at the pointer instead. The
//! desktop app turns it on (`PdfCraftApp::own_crosshair`); tests see the system cursor as before.

use egui::{Color32, Id, LayerId, Order, Pos2, Stroke, vec2};

/// How far each arm reaches from the centre, and the gap left at the centre, in points.
const ARM: f32 = 11.0;
const GAP: f32 = 2.0;

impl crate::PdfCraftApp {
    /// After the frame: a crosshair about to be shown is drawn by the app instead (module notes).
    pub(crate) fn draw_own_crosshair(&self, ctx: &egui::Context) {
        if !self.own_crosshair || ctx.output(|o| o.cursor_icon) != egui::CursorIcon::Crosshair {
            return;
        }
        let Some(at) = ctx.pointer_hover_pos() else { return };
        ctx.set_cursor_icon(egui::CursorIcon::None);
        let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("maewpdf-crosshair")));
        // Whole pixels, so the 1-pixel black line is sharp.
        let ppp = ctx.pixels_per_point();
        let snap = |p: Pos2| Pos2::new((p.x * ppp).round() / ppp + 0.5 / ppp, (p.y * ppp).round() / ppp + 0.5 / ppp);
        let c = snap(at);
        let arms = [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)];
        for (width, colour) in [(3.0 / ppp.max(1.0) + 1.0, Color32::WHITE), (1.5, Color32::BLACK)] {
            for d in arms {
                painter.line_segment([c + d * GAP, c + d * ARM], Stroke::new(width, colour));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::PdfCraftApp;

    /// On, a crosshair claim hides the system cursor and is drawn by the app; off (as in tests and
    /// for any other cursor), the cursor is left as it is.
    #[test]
    fn a_crosshair_is_drawn_by_the_app_when_it_is_on() {
        let ctx = egui::Context::default();
        let frame = |app: &PdfCraftApp, icon: egui::CursorIcon| {
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::PointerMoved(egui::pos2(50.0, 50.0)));
            let mut out = ctx.run_ui(input, |ui| {
                ui.ctx().set_cursor_icon(icon);
                app.draw_own_crosshair(ui.ctx());
            });
            // An input-only test has no renderer to take the font texture.
            out.textures_delta.clear();
            out.platform_output.cursor_icon
        };
        let mut app = PdfCraftApp::new();
        assert_eq!(frame(&app, egui::CursorIcon::Crosshair), egui::CursorIcon::Crosshair, "off by default");
        app.own_crosshair = true;
        assert_eq!(frame(&app, egui::CursorIcon::Crosshair), egui::CursorIcon::None, "drawn by the app");
        assert_eq!(frame(&app, egui::CursorIcon::PointingHand), egui::CursorIcon::PointingHand, "other cursors are the system's");
    }
}
