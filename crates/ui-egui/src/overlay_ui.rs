//! Overlay Pages: choose the two colours and, optionally, line the versions up by clicking the
//! same 1–3 features on each (the Align Point tool), then make the overlay as a new PDF (see
//! `pdfcraft_organize::overlay`).

use egui::{Align, Color32, Layout};
use pdfcraft_engine::DocId;
use pdfcraft_engine::compare::OverlayOptions;

use crate::theme::{self, Tokens};
use crate::{Dialog, PdfCraftApp, QuickTool, widgets};

/// Which version points are being picked on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Old,
    New,
}

/// The Overlay Pages choices, kept while points are picked on the pages.
pub struct OverlaySetup {
    pub old: DocId,
    pub new: DocId,
    pub old_colour: Color32,
    pub new_colour: Color32,
    /// Line the versions up by matching points (otherwise their lower-left corners meet).
    pub align: bool,
    /// Points to pick on each version (1–3).
    pub count: usize,
    /// Points picked: (page, view-space point), on one page per version.
    pub old_points: Vec<(usize, [f32; 2])>,
    pub new_points: Vec<(usize, [f32; 2])>,
    /// The version whose points the Align Point tool is picking.
    pub picking: Option<Side>,
}

/// Picked points are drawn in these colours on the page.
pub const MARK: Color32 = Color32::from_rgb(0xE0, 0x1E, 0x5A);

impl OverlaySetup {
    pub fn new(old: DocId, new: DocId) -> Self {
        OverlaySetup {
            old,
            new,
            old_colour: Color32::from_rgb(0xFF, 0, 0),
            new_colour: Color32::from_rgb(0, 0, 0xFF),
            align: false,
            count: 2,
            old_points: Vec::new(),
            new_points: Vec::new(),
            picking: None,
        }
    }

    fn picked(&self) -> bool {
        self.old_points.len() == self.count && self.new_points.len() == self.count
    }

    /// The new pages' placement: lower-left corners together, or fitted to the picked points
    /// (`None` while points are still missing).
    pub fn placement(&self, app: &PdfCraftApp) -> Option<Result<[f64; 6], String>> {
        if !self.align {
            return Some(Ok(OverlayOptions::default().new_transform));
        }
        if !self.picked() {
            return None;
        }
        let side = |pts: &[(usize, [f32; 2])]| -> (usize, Vec<[f64; 2]>) {
            (pts.first().map_or(0, |p| p.0), pts.iter().map(|(_, p)| [p[0] as f64, p[1] as f64]).collect())
        };
        let ((op, o), (np, n)) = (side(&self.old_points), side(&self.new_points));
        Some(app.session.overlay_alignment((self.old, op, &o), (self.new, np, &n)).map_err(|e| e.to_string()))
    }

    fn options(&self, placement: [f64; 6]) -> OverlayOptions {
        let rgb = |c: Color32| [c.r() as f64 / 255.0, c.g() as f64 / 255.0, c.b() as f64 / 255.0];
        OverlayOptions { old_colour: rgb(self.old_colour), new_colour: rgb(self.new_colour), new_transform: placement, ..Default::default() }
    }
}

/// A placement as words: shift, turn and scale (or "stretched" when the scale differs by
/// direction).
fn describe(m: [f64; 6]) -> String {
    let fmt1 = |v: f64| format!("{:.1}", if v.abs() < 0.05 { 0.0 } else { v });
    let (sx, sy) = ((m[0] * m[0] + m[1] * m[1]).sqrt(), (m[2] * m[2] + m[3] * m[3]).sqrt());
    let turn = m[1].atan2(m[0]).to_degrees();
    let mut s = crate::i18n::fmt(tl!("Shift {x}, {y} pt"), &[("x", &fmt1(m[4])), ("y", &fmt1(m[5]))]);
    if turn.abs() >= 0.05 {
        s += &crate::i18n::fmt(tl!(" · turn {a}°"), &[("a", &fmt1(turn))]);
    }
    if (sx - sy).abs() > 1e-3 * sx.max(sy) {
        s += &crate::i18n::fmt(tl!(" · scale {x}% across, {y}% down"), &[("x", &fmt1(sx * 100.0)), ("y", &fmt1(sy * 100.0))]);
    } else if (sx - 1.0).abs() >= 5e-4 {
        s += &crate::i18n::fmt(tl!(" · scale {s}%"), &[("s", &fmt1(sx * 100.0))]);
    }
    s
}

/// What the dialog asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogAction {
    Create,
    Pick,
    Cancel,
}

/// The Overlay Pages dialog.
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens) -> Option<DialogAction> {
    ui.label(egui::RichText::new(tl!("Overlay Pages")).font(theme::semibold(18.0)));
    ui.add_space(8.0);
    let Some(setup) = app.overlay.as_ref() else { return Some(DialogAction::Cancel) };
    let name = |id: DocId| app.session.get(id).map(|d| d.name.clone());
    let (Some(old_name), Some(new_name)) = (name(setup.old), name(setup.new)) else { return Some(DialogAction::Cancel) };
    let placement = setup.placement(app);
    let Some(setup) = app.overlay.as_mut() else { return Some(DialogAction::Cancel) };
    let mut action = None;
    egui::Grid::new("overlay-files").num_columns(3).spacing([12.0, 10.0]).show(ui, |ui| {
        ui.label(tl!("Old file:"));
        ui.label(egui::RichText::new(old_name).strong());
        egui::color_picker::color_edit_button_srgba(ui, &mut setup.old_colour, egui::color_picker::Alpha::Opaque);
        ui.end_row();
        ui.label(tl!("New file:"));
        ui.label(egui::RichText::new(new_name).strong());
        egui::color_picker::color_edit_button_srgba(ui, &mut setup.new_colour, egui::color_picker::Alpha::Opaque);
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Lines both versions share come out dark; lines only one has keep its colour.")).small().color(t.text_muted));
    ui.add_space(10.0);
    ui.checkbox(&mut setup.align, tl!("Line up by matching points"));
    if setup.align {
        ui.indent("overlay-align", |ui| {
            ui.horizontal(|ui| {
                ui.label(tl!("Points on each file:"));
                for n in 1..=3 {
                    if ui.selectable_label(setup.count == n, n.to_string()).clicked() && setup.count != n {
                        setup.count = n;
                        setup.old_points.clear();
                        setup.new_points.clear();
                    }
                }
            });
            let hint = match setup.count {
                1 => tl!("One point shifts the new pages."),
                2 => tl!("Two points shift, turn and scale the new pages evenly."),
                _ => tl!("Three points also fit a sheet stretched more one way than the other."),
            };
            ui.label(egui::RichText::new(hint).small().color(t.text_muted));
            let picked = |n: usize| crate::i18n::fmt(tl!("{n} of {count} points"), &[("n", &n.to_string()), ("count", &setup.count.to_string())]);
            egui::Grid::new("overlay-points").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                ui.label(tl!("Old file:"));
                ui.label(picked(setup.old_points.len()));
                ui.end_row();
                ui.label(tl!("New file:"));
                ui.label(picked(setup.new_points.len()));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button(tl!("Pick points…")).clicked() {
                    action = Some(DialogAction::Pick);
                }
                if ui.add_enabled(!setup.old_points.is_empty() || !setup.new_points.is_empty(), egui::Button::new(tl!("Clear points"))).clicked() {
                    setup.old_points.clear();
                    setup.new_points.clear();
                }
            });
            match &placement {
                Some(Ok(m)) => {
                    ui.label(egui::RichText::new(describe(*m)).color(t.text_muted));
                }
                Some(Err(e)) => {
                    let red = ui.visuals().error_fg_color;
                    ui.label(egui::RichText::new(e).color(red));
                }
                None => {}
            }
        });
    }
    ui.add_space(10.0);
    // The placement shown is for the points as they were when this frame started.
    let can_create = !setup.align || (setup.picked() && matches!(placement, Some(Ok(_))));
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled_ui(can_create, |ui| widgets::pill_button(ui, tl!("Create overlay"), true)).inner.clicked() {
                action = Some(DialogAction::Create);
            }
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                action = Some(DialogAction::Cancel);
            }
        })
    });
    action
}

impl PdfCraftApp {
    /// Compare panel ▸ Overlay pages: the dialog, keeping earlier choices for the same pair.
    pub fn open_overlay_dialog(&mut self, old: DocId, new: DocId) {
        if self.overlay.as_ref().is_none_or(|s| s.old != old || s.new != new) {
            self.overlay = Some(OverlaySetup::new(old, new));
        }
        self.dialog = Some(Dialog::OverlayPages);
    }

    pub(crate) fn overlay_dialog_action(&mut self, action: DialogAction) {
        match action {
            DialogAction::Create => self.create_overlay(),
            DialogAction::Pick => self.start_align_picking(),
            DialogAction::Cancel => {}
        }
    }

    /// Make the overlay with the dialog's choices and open it.
    pub fn create_overlay(&mut self) {
        let Some(setup) = self.overlay.as_ref() else { return };
        let placement = match setup.placement(self) {
            Some(Ok(m)) => m,
            Some(Err(e)) => return self.notify_error(e),
            None => return self.notify_tr("Pick the alignment points on both files first"),
        };
        let opts = setup.options(placement);
        match self.session.compare_overlay(setup.old, setup.new, &opts) {
            Ok(bytes) => {
                if let Err(e) = self.open_bytes("Overlay.pdf", None, bytes.to_vec()) {
                    self.notify_error(e);
                }
            }
            Err(e) => self.notify_error(e),
        }
    }

    /// Start the Align Point tool on the old version.
    pub fn start_align_picking(&mut self) {
        let Some(setup) = self.overlay.as_mut() else { return };
        setup.align = true;
        setup.old_points.clear();
        setup.new_points.clear();
        setup.picking = Some(Side::Old);
        self.quick_tool = QuickTool::AlignPoint;
        self.show_align_side(Side::Old);
    }

    /// Show the version being picked on, its points so far, and what to click next.
    fn show_align_side(&mut self, side: Side) {
        let Some(setup) = self.overlay.as_ref() else { return };
        let (id, points, count) = match side {
            Side::Old => (setup.old, setup.old_points.clone(), setup.count),
            Side::New => (setup.new, setup.new_points.clone(), setup.count),
        };
        for v in &mut self.views {
            v.align_marks = if v.id == id { points.clone() } else { Vec::new() };
        }
        if let Some(i) = self.views.iter().position(|v| v.id == id) {
            self.active = Some(i);
        }
        let next = (points.len() + 1).to_string();
        let msg = match side {
            Side::Old => tl!("Click point {i} of {n} on the old file (Esc cancels)"),
            Side::New => tl!("Click point {i} of {n} on the new file, the same features in the same order (Esc cancels)"),
        };
        self.notify(crate::i18n::fmt(msg, &[("i", &next), ("n", &count.to_string())]));
    }

    /// The Align Point tool was clicked at `at` (view space) on `page` of view `index`.
    pub fn align_click(&mut self, index: usize, page: usize, at: [f32; 2]) {
        let Some(id) = self.views.get(index).map(|v| v.id) else { return };
        let Some(setup) = self.overlay.as_mut() else { return };
        let Some(side) = setup.picking else { return };
        let (want, points) = match side {
            Side::Old => (setup.old, &mut setup.old_points),
            Side::New => (setup.new, &mut setup.new_points),
        };
        if id != want {
            // Picking moved to another tab: bring the right file back.
            return self.show_align_side(side);
        }
        if let Some((first, _)) = points.first()
            && *first != page
        {
            let p = (first + 1).to_string();
            return self.notify_fmt("Pick all the points on page {p}", &[("p", &p)]);
        }
        points.push((page, at));
        let done = points.len() >= setup.count;
        match (side, done) {
            (_, false) => self.show_align_side(side),
            (Side::Old, true) => {
                setup.picking = Some(Side::New);
                self.show_align_side(Side::New);
            }
            (Side::New, true) => {
                setup.picking = None;
                self.finish_align_picking();
                self.dialog = Some(Dialog::OverlayPages);
            }
        }
    }

    /// Esc while picking: stop, keeping the points picked so far.
    pub(crate) fn cancel_align_picking(&mut self) {
        if let Some(s) = self.overlay.as_mut() {
            s.picking = None;
        }
        self.finish_align_picking();
        self.notify_tr("Stopped picking alignment points");
    }

    fn finish_align_picking(&mut self) {
        self.quick_tool = QuickTool::Select;
        for v in &mut self.views {
            v.align_marks.clear();
        }
        if let Some(i) = self.overlay.as_ref().and_then(|s| self.views.iter().position(|v| v.id == s.new)) {
            self.active = Some(i);
        }
    }
}

/// The Align Point tool on page `page`: a click records the point; picked points are drawn
/// numbered.
pub(crate) fn page_input(ui: &egui::Ui, resp: &egui::Response, xf: &crate::canvas::PageXform, page: usize, view: &mut crate::canvas::DocView) {
    if let Some(p) = ui.input(|i| i.pointer.hover_pos()).filter(|p| xf.rect.contains(*p)) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if resp.clicked() {
            let (x, y) = xf.screen_to_view(p);
            view.align_click = Some((page, [x, y]));
        }
    }
}

/// Picked alignment points on page `page`, numbered.
pub(crate) fn paint_marks(painter: &egui::Painter, xf: &crate::canvas::PageXform, page: usize, view: &crate::canvas::DocView) {
    for (n, (_, p)) in view.align_marks.iter().enumerate().filter(|(_, (pg, _))| *pg == page) {
        let c = xf.view_rect([p[0], p[1], p[0], p[1]]).center();
        let stroke = egui::Stroke::new(1.5, MARK);
        painter.circle_stroke(c, 7.0, stroke);
        painter.line_segment([c - egui::vec2(11.0, 0.0), c + egui::vec2(11.0, 0.0)], stroke);
        painter.line_segment([c - egui::vec2(0.0, 11.0), c + egui::vec2(0.0, 11.0)], stroke);
        painter.text(c + egui::vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, (n + 1).to_string(), theme::semibold(12.0), MARK);
    }
}
