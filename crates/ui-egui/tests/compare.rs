//! Compare files in the real shell (egui_kittest): the dialog, the Compare panel, marks and
//! the report.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Session;
use pdfcraft_ui_egui::{PdfCraftApp, QuickTool, RightPanel};

#[test]
fn compare_two_versions() {
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    h.get_by_label("Compare Files");
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert_eq!(h.state().right, Some(RightPanel::Compare));
    assert_eq!(h.state().toast.clone().unwrap().0, "2 differences found");
    h.get_by_label("2 Replaced");
    h.get_by_label("\"five\" → \"three\"").click();
    h.run_steps(2);
    assert_eq!(h.state().compare.as_ref().unwrap().selected, Some(0));
    let id = h.state().active_ids().unwrap().1;
    let i = h.state().active_ids().unwrap().0;
    assert_eq!(h.state().views[i].compare_marks.len(), 2);
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(id).unwrap().info.annotations.len(), 2);
    h.get_by_label("Overlay pages").click();
    h.run_steps(3);
    h.get_by_label("Overlay Pages");
    h.get_by_label("Lower-left corners together").click();
    h.run_steps(2);
    h.get_by_label("Create overlay").click();
    h.run_steps(3);
    let overlay = h.state().session.get(h.state().active_ids().unwrap().1).unwrap();
    assert_eq!(overlay.name, "Overlay.pdf");
    let layers: Vec<&str> = overlay.info.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(layers, ["Old", "New"]);
    // Back to the compared document, whose panel offers the report.
    let back = h.state().views.iter().position(|v| v.id == id).unwrap();
    h.state_mut().active = Some(back);
    h.run_steps(2);
    h.get_by_label("Report…").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(h.state().active_ids().unwrap().1).unwrap().name, "Compare Report.pdf");
}

#[test]
fn overlay_aligned_by_picked_points() {
    let s = Session::new();
    let v1 = s.create_from_text("t", "Level 1 floor plan").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Level 1 floor plan, revised").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    h.get_by_label("Compare").click();
    h.run_steps(3);
    h.get_by_label("Overlay pages").click();
    h.run_steps(3);
    h.get_by_label("By matching points").click();
    h.run_steps(2);
    h.get_by_label("Pick points…").click();
    h.run_steps(3);
    let (old_view, new_view) = (0, 1);
    assert_eq!(h.state().quick_tool, QuickTool::AlignPoint);
    assert_eq!(h.state().active, Some(old_view), "picking starts on the old file");
    assert!(h.state().dialog.is_none());
    // Two features on the old sheet, then the same two on the new one, 10 pt right and 5 pt
    // lower (view space, y down).
    h.state_mut().align_click(old_view, 0, [100.0, 100.0]);
    h.state_mut().align_click(old_view, 0, [300.0, 100.0]);
    h.run_steps(2);
    assert_eq!(h.state().active, Some(new_view), "then the new file");
    assert_eq!(h.state().views[old_view].align_marks.len(), 0);
    // A click on another page is refused: the points of a version share one page.
    h.state_mut().align_click(new_view, 0, [110.0, 105.0]);
    h.state_mut().align_click(new_view, 1, [310.0, 105.0]);
    assert_eq!(h.state().views[new_view].align_marks.len(), 1, "the first point is drawn, the other page's refused");
    h.state_mut().align_click(new_view, 0, [310.0, 105.0]);
    h.run_steps(3);
    assert_eq!(h.state().quick_tool, QuickTool::Select);
    h.get_by_label("Overlay Pages");
    h.get_by_label("Shift -10.0, 5.0 pt");
    h.get_by_label("Create overlay").click();
    h.run_steps(3);
    let overlay = h.state().session.get(h.state().active_ids().unwrap().1).unwrap();
    assert_eq!(overlay.name, "Overlay.pdf");
}

/// A one-page US Letter PDF drawing `content`.
fn drawing(content: &str) -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    pdf
}

/// A floor plan: rooms, a diagonal and a slanted wall, 2 pt lines.
const PLAN: &str = "2 w 60 92 m 520 92 l 520 742 l 60 742 l h S 60 562 m 250 562 l 250 742 l S 250 462 m 520 462 l S \
                    330 92 m 330 362 l S 395 182 m 470 182 l 470 272 l 395 272 l h S 330 362 m 520 462 l S 90 392 m 210 532 l S";

#[test]
fn overlay_lines_up_automatically() {
    // The new sheet: the plan moved 12 pt right and 7 pt down, with a revision.
    let v1 = drawing(PLAN);
    let v2 = drawing(&format!("q 1 0 0 1 12 -7 cm {PLAN} Q 2 w 100 150 m 300 250 l S"));
    let blank = drawing("");
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("blank.pdf", None, blank.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    let (old, blank, new) = (h.state().views[0].id, h.state().views[1].id, h.state().views[2].id);
    // Automatic is the default: the dialog finds the shift as it opens.
    h.state_mut().open_overlay_dialog(old, new);
    h.run_steps(3);
    let found = h.state().overlay.as_ref().unwrap().auto.as_ref().unwrap().1.clone().unwrap();
    let m = found.transform;
    assert!((m[4] + 12.0).abs() < 0.6 && (m[5] - 7.0).abs() < 0.6 && (m[0] - 1.0).abs() < 0.002, "{found:?}");
    h.get_by_label_contains("of the lines match");
    h.get_by_label("Create overlay").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(h.state().active_ids().unwrap().1).unwrap().name, "Overlay.pdf");

    // A blank page can't be lined up: the reason shows and Create stays off until another
    // choice is made.
    h.state_mut().open_overlay_dialog(blank, new);
    h.run_steps(3);
    h.get_by_label_contains("too little drawn");
    let before = h.state().views.len();
    h.get_by_label("Create overlay").click();
    h.run_steps(2);
    assert_eq!(h.state().views.len(), before, "nothing was made");
    h.get_by_label("Lower-left corners together").click();
    h.run_steps(2);
    h.get_by_label("Create overlay").click();
    h.run_steps(3);
    assert_eq!(h.state().views.len(), before + 1);
}

#[test]
fn picked_points_snap_to_line_ends() {
    // An L of two lines with its corner at (100, 600); the new sheet draws it 10 pt right and
    // 5 pt lower. In view space (y down from the top of a 792 pt page) the old corner is at
    // (100, 192) and the old line's far end at (300, 192); the new ones at (110, 197) and
    // (310, 197).
    let v1 = drawing("1 w 100 600 m 300 600 l S 100 600 m 100 400 l S");
    let v2 = drawing("1 w 110 595 m 310 595 l S 110 595 m 110 395 l S");
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    h.get_by_label("Compare").click();
    h.run_steps(3);
    h.get_by_label("Overlay pages").click();
    h.run_steps(3);
    h.get_by_label("By matching points").click();
    h.run_steps(2);
    assert!(h.state().overlay.as_ref().unwrap().snap, "snapping is on by default");
    h.get_by_label("Pick points…").click();
    h.run_steps(2);
    // Clicks a couple of points off each line end land on it exactly.
    h.state_mut().align_click(0, 0, [102.0, 190.0]);
    h.state_mut().align_click(0, 0, [298.0, 193.5]);
    h.state_mut().align_click(1, 0, [112.5, 199.0]);
    assert_eq!(h.state().views[1].align_marks, [(0, [110.0, 197.0])], "snapped to the new corner");
    h.state_mut().align_click(1, 0, [308.0, 196.0]);
    h.run_steps(3);
    let s = h.state().overlay.as_ref().unwrap();
    assert_eq!(s.old_points, [(0, [100.0, 192.0]), (0, [300.0, 192.0])]);
    assert_eq!(s.new_points, [(0, [110.0, 197.0]), (0, [310.0, 197.0])]);
    h.get_by_label("Shift -10.0, 5.0 pt");

    // Far from any line nothing snaps; with snapping off, nothing snaps even next to one.
    h.get_by_label("Pick points…").click();
    h.run_steps(2);
    h.state_mut().align_click(0, 0, [500.0, 700.0]);
    h.state_mut().overlay.as_mut().unwrap().snap = false;
    h.state_mut().align_click(0, 0, [102.0, 190.0]);
    assert_eq!(h.state().overlay.as_ref().unwrap().old_points, [(0, [500.0, 700.0]), (0, [102.0, 190.0])]);
}

/// Writes a screenshot of the Compare panel when PDFCRAFT_SHOT is set (for review).
#[test]
fn compare_panel_screenshot() {
    let Ok(out) = std::env::var("PDFCRAFT_SHOT") else { return };
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    h.state_mut().run_compare();
    for _ in 0..20 {
        h.run_steps(2);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    h.render().unwrap().save(out).unwrap();
}

/// Writes screenshots of Overlay Pages when PDFCRAFT_SHOT is set (for review): picking points
/// (`…-pick.png`), the dialog lining up automatically (`…-auto.png`) and with the fitted points
/// (`…-dialog.png`), and the overlay
/// (`…-result.png`).
#[test]
fn overlay_screenshots() {
    let Ok(out) = std::env::var("PDFCRAFT_SHOT") else { return };
    let name = |part: &str| out.trim_end_matches(".png").to_string() + "-" + part + ".png";
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    let settle = |h: &mut Harness<PdfCraftApp>| {
        for _ in 0..20 {
            h.run_steps(2);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    };
    h.run_steps(4);
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    h.state_mut().run_compare();
    h.run_steps(3);
    h.get_by_label("Overlay pages").click();
    settle(&mut h);
    h.render().unwrap().save(name("auto")).unwrap();
    h.get_by_label("By matching points").click();
    h.run_steps(2);
    h.get_by_label("Pick points…").click();
    h.run_steps(2);
    h.state_mut().align_click(0, 0, [72.0, 72.0]);
    settle(&mut h);
    h.render().unwrap().save(name("pick")).unwrap();
    h.state_mut().align_click(0, 0, [400.0, 72.0]);
    h.state_mut().align_click(1, 0, [74.0, 70.0]);
    h.state_mut().align_click(1, 0, [402.0, 71.0]);
    settle(&mut h);
    h.render().unwrap().save(name("dialog")).unwrap();
    h.get_by_label("Create overlay").click();
    settle(&mut h);
    h.render().unwrap().save(name("result")).unwrap();
}

#[test]
fn pdfa_dialog_verifies_and_converts() {
    let doc = Session::new().create_from_text("t", "Keep forever").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("keep.pdf", None, doc.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("standards.pdfa"));
    h.run_steps(2);
    h.get_by_label("Declared conformance: none");
    h.get_by_label("Verify").click();
    h.run_steps(3);
    h.get_by_label("The document has no XMP metadata");
    h.get_by_label("Save as PDF/A").click();
    h.run_steps(3);
    h.get_by_label("Declared conformance: PDF/A-2b");
    let issues = h.state().pdfa.issues.clone().unwrap();
    assert!(issues.iter().all(|i| !i.fixable), "{issues:?}");
    let id = h.state().active_ids().unwrap().1;
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), Some("Save as PDF/A-2b"));
}

#[test]
fn export_to_word_html_and_rtf() {
    let doc = Session::new().create_from_text("t", "Exported words").unwrap().to_vec();
    let dir = std::env::temp_dir().join(format!("pdfcraft-office-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.open_bytes("e.pdf", None, doc).unwrap();
    for ext in ["docx", "html", "rtf"] {
        let out = dir.join(format!("e.{ext}"));
        app.save_override = Some(out.to_string_lossy().into_owned());
        assert!(app.execute(&format!("export.{ext}")));
        assert!(std::fs::metadata(&out).unwrap().len() > 40, "{ext}");
    }
    assert!(std::fs::read_to_string(dir.join("e.html")).unwrap().contains("Exported words"));
}
