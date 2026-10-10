//! Compare files: the text differences between two open documents, a report, and the
//! differences marked as comments in the newer one.

use std::sync::Arc;

pub use pdfcraft_compare::{Change, Comparison, Kind, Side};
pub use pdfcraft_organize::OverlayOptions;

use crate::{DocId, Edit, EditError, Markup, NewAnnotation, NoteIcon, Session, Shape};

/// What automatic alignment found: the new pages' placement and how much ink it lines up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoAlignment {
    pub transform: [f64; 6],
    /// The share of the new page's ink that lands on the old page's ink, 0–1.
    pub score: f64,
}

/// Automatic alignment renders the old page at about this many pixels on its longer side.
pub(crate) const ALIGN_SIDE: f32 = 2000.0;

/// Below this share of ink lined up, automatic alignment gives up.
const MIN_ALIGN_SCORE: f64 = 0.5;

/// Acrobat's compare colours: replaced blue, inserted green, deleted red.
pub fn colour(kind: Kind) -> crate::Rgb {
    match kind {
        Kind::Replaced => [0.2, 0.45, 0.95],
        Kind::Inserted => [0.2, 0.75, 0.3],
        Kind::Deleted => [0.9, 0.25, 0.25],
    }
}

impl crate::Document {
    /// Standards ▸ Verify PDF/A: the rules the document breaks for `level`.
    pub fn pdfa_verify(&self, level: pdfcraft_preflight::Level) -> Vec<pdfcraft_preflight::Issue> {
        self.editor.as_ref().map(|e| pdfcraft_preflight::verify(&e.cos, level)).unwrap_or_default()
    }

    /// The standards the document declares (Standards panel).
    pub fn standards(&self) -> pdfcraft_preflight::Declared {
        self.editor.as_ref().map(|e| pdfcraft_preflight::declared(&e.cos)).unwrap_or_default()
    }

    /// Every word of the document in reading order, with page and box.
    pub fn words(&self) -> Vec<pdfcraft_compare::Word> {
        let config = pdfcraft_render::RenderConfig { password: self.password.as_deref().map(Arc::from), ..Default::default() };
        let mut r = pdfcraft_render::PageRenderer::new(self.bytes.clone(), config);
        let mut out = Vec::new();
        for (page, info) in self.info.pages.iter().enumerate() {
            let res = r.render(pdfcraft_render::RenderRequest { page, kind: pdfcraft_render::RequestKind::Text, scale: 1.0, ..Default::default() });
            if let Some(t) = res.text {
                out.extend(crate::js::page_words(&t, info).into_iter().map(|(text, rect)| pdfcraft_compare::Word { text, page, rect }));
            }
        }
        out
    }
}

impl Session {
    /// Compare files: the text differences from document `old` to document `new`.
    pub fn compare(&self, old: DocId, new: DocId) -> Result<Comparison, EditError> {
        let a = self.get(old).ok_or(EditError::NoDocument)?;
        let b = self.get(new).ok_or(EditError::NoDocument)?;
        Ok(pdfcraft_compare::compare(&a.words(), &b.words()))
    }

    /// Visual compare: regions where page n of `new` looks different from page n of `old`
    /// (rendered at `dpi`), as (page, user-space box in `new`).
    pub fn compare_visual(&self, old: DocId, new: DocId, dpi: f32) -> Result<Vec<(usize, [f64; 4])>, EditError> {
        let a = self.get(old).ok_or(EditError::NoDocument)?;
        let b = self.get(new).ok_or(EditError::NoDocument)?;
        let renderer = |d: &crate::Document| {
            pdfcraft_render::PageRenderer::new(
                d.bytes.clone(),
                pdfcraft_render::RenderConfig { password: d.password.as_deref().map(Arc::from), ..Default::default() },
            )
        };
        let (mut ra, mut rb) = (renderer(a), renderer(b));
        let scale = dpi.clamp(18.0, 150.0) / 72.0;
        let mut out = Vec::new();
        for page in 0..a.info.pages.len().min(b.info.pages.len()) {
            let req = pdfcraft_render::RenderRequest { page, scale, ..Default::default() };
            let (x, y) = (ra.render(req), rb.render(req));
            if x.error.is_some() || y.error.is_some() {
                continue;
            }
            let info = &b.info.pages[page];
            let s = y.width as f32 / info.width.max(1e-3);
            for r in pdfcraft_compare::visual_regions((&y.rgba, y.width, y.height), (&x.rgba, x.width, x.height), 24) {
                let p = info.view_to_user(r[0] as f32 / s, r[1] as f32 / s);
                let q = info.view_to_user(r[2] as f32 / s, r[3] as f32 / s);
                out.push((page, [p[0].min(q[0]) as f64, p[1].min(q[1]) as f64, p[0].max(q[0]) as f64, p[1].max(q[1]) as f64]));
            }
        }
        Ok(out)
    }

    /// Overlay Pages: a new PDF (not opened) laying page n of `new` over page n of `old`, old in
    /// one colour and new in another, so lines only one version has stand out. Each version is a
    /// layer. See [`pdfcraft_organize::overlay`].
    pub fn compare_overlay(&self, old: DocId, new: DocId, opts: &OverlayOptions) -> Result<Arc<Vec<u8>>, EditError> {
        let out = pdfcraft_organize::overlay(self.cos(old)?, self.cos(new)?, opts)?;
        self.write_new(&out)
    }

    /// Alignment for Overlay Pages from matching points picked on the pages as displayed (view
    /// space: points from the page's top-left corner, as the canvas and the tools use): one to
    /// three points on page `old_page` of `old` and the same number on page `new_page` of `new`.
    /// Returns the placement for [`OverlayOptions::new_transform`]; see
    /// [`pdfcraft_organize::alignment`].
    pub fn overlay_alignment(
        &self,
        (old, old_page, old_points): (DocId, usize, &[[f64; 2]]),
        (new, new_page, new_points): (DocId, usize, &[[f64; 2]]),
    ) -> Result<[f64; 6], EditError> {
        // The overlay places pages upright from their lower-left corner: flip y.
        let flip = |id: DocId, page: usize, pts: &[[f64; 2]]| -> Result<Vec<[f64; 2]>, EditError> {
            let doc = self.get(id).ok_or(EditError::NoDocument)?;
            let info = doc.info.pages.get(page).ok_or(pdfcraft_organize::OrganizeError::NoSuchPage(page))?;
            Ok(pts.iter().map(|p| [p[0], info.height as f64 - p[1]]).collect())
        };
        let (o, n) = (flip(old, old_page, old_points)?, flip(new, new_page, new_points)?);
        Ok(pdfcraft_organize::alignment(&o, &n)?)
    }

    /// Automatic alignment for Overlay Pages: renders page `old_page` of `old` and `new_page` of
    /// `new` at the same resolution and finds the shift, scale and slight turn that best line
    /// the new one up with the old (see [`pdfcraft_compare::auto_align`]). Returns the placement
    /// for [`OverlayOptions::new_transform`] and the share of the new page's ink that then lands
    /// on the old page's (0–1). Fails when either page is blank, or no placement lines up even
    /// half of the ink (different sheets, or turned too far: pick points instead).
    pub fn overlay_auto_alignment(&self, (old, old_page): (DocId, usize), (new, new_page): (DocId, usize)) -> Result<AutoAlignment, EditError> {
        // About ALIGN_SIDE pixels along the old page's longer side; the new page at the same
        // resolution, so a difference in plotting scale shows.
        let size = self.get(old).and_then(|d| d.info.pages.get(old_page)).map_or(792.0, |p| p.width.max(p.height).max(1.0));
        let scale = (ALIGN_SIDE / size).clamp(0.1, 4.0);
        let (a, ka, _, ha) = self.render_for_compare(old, old_page, scale)?;
        let (b, kb, _, hb) = self.render_for_compare(new, new_page, scale)?;
        let bad = |why: &str| EditError::from(pdfcraft_organize::OrganizeError::Invalid(why.to_string()));
        let fit = pdfcraft_compare::auto_align((&a.rgba, a.width, a.height), (&b.rgba, b.width, b.height))
            .ok_or_else(|| bad("a page has too little drawn on it to line up automatically"))?;
        if fit.score < MIN_ALIGN_SCORE {
            return Err(bad("the pages don't line up automatically (are they the same sheet?): pick matching points instead"));
        }
        // Three corners of the new page (overlay space: points, y up), carried through the fit
        // in pixels (y down) to the old page, fix the placement exactly.
        let new_pts = [[0.0, 0.0], [f64::from(b.width) / kb, 0.0], [0.0, hb]];
        let old_pts = new_pts.map(|[x, y]| {
            let [px, py] = fit.apply([x * kb, (hb - y) * kb]);
            [px / ka, ha - py / ka]
        });
        let transform = pdfcraft_organize::alignment(&old_pts, &new_pts)?;
        Ok(AutoAlignment { transform, score: fit.score })
    }

    /// Page `page` of `id` rendered at `scale` for alignment and change finding, with the
    /// pixels per point it was actually drawn at (the renderer may cap very large pages) and the
    /// page's displayed width and height in points.
    pub(crate) fn render_for_compare(&self, id: DocId, page: usize, scale: f32) -> Result<(pdfcraft_render::RenderedPage, f64, f64, f64), EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        let info = doc.info.pages.get(page).ok_or(pdfcraft_organize::OrganizeError::NoSuchPage(page))?;
        let mut r = pdfcraft_render::PageRenderer::new(
            doc.bytes.clone(),
            pdfcraft_render::RenderConfig { password: doc.password.as_deref().map(Arc::from), ..Default::default() },
        );
        let out = r.render(pdfcraft_render::RenderRequest { page, scale, ..Default::default() });
        if let Some(e) = out.error.clone() {
            return Err(pdfcraft_organize::OrganizeError::Invalid(format!("page {} of {} couldn't be drawn: {e}", page + 1, doc.name)).into());
        }
        let k = f64::from(out.width) / f64::from(info.width.max(1e-3));
        if !(k > 0.0 && k.is_finite()) {
            return Err(pdfcraft_organize::OrganizeError::Invalid(format!("page {} of {} couldn't be drawn", page + 1, doc.name)).into());
        }
        Ok((out, k, f64::from(info.width), f64::from(info.height)))
    }

    /// The compare report as a new PDF (not opened).
    pub fn compare_report(&self, old: DocId, new: DocId) -> Result<Arc<Vec<u8>>, EditError> {
        let c = self.compare(old, new)?;
        let name = |id| self.get(id).map(|d| d.name.clone()).unwrap_or_default();
        self.create_from_text("Compare Report", &pdfcraft_compare::report(&c, &name(old), &name(new)))
    }

    /// Mark the differences in `new` as comments: highlights over replaced and inserted text
    /// (blue, green) and a note where text was deleted (red), authored "Compare". One undoable
    /// step; returns how many comments were added.
    pub fn mark_differences(&mut self, old: DocId, new: DocId) -> Result<usize, EditError> {
        let c = self.compare(old, new)?;
        let mut edits = Vec::new();
        for ch in &c.changes {
            let color = colour(ch.kind);
            let contents = match ch.kind {
                Kind::Replaced => format!("Replaced: \"{}\" with \"{}\"", ch.old.text, ch.new.text),
                Kind::Inserted => format!("Inserted: \"{}\"", ch.new.text),
                Kind::Deleted => format!("Deleted: \"{}\"", ch.old.text),
            };
            let shape = if ch.new.rects.is_empty() {
                let Some(r) = ch.new.near else { continue };
                Shape::Note { at: [r[0], r[3]], icon: NoteIcon::Note }
            } else {
                Shape::TextMarkup {
                    kind: Markup::Highlight,
                    quads: ch.new.rects.iter().map(|r| [r[0], r[3], r[2], r[3], r[0], r[1], r[2], r[1]]).collect(),
                }
            };
            let mut style = crate::Style::default_for(&shape);
            style.color = color;
            edits.push(Edit::AddAnnotation(NewAnnotation { page: ch.new.page, shape, style, contents, author: "Compare".into() }));
        }
        let n = edits.len();
        if n > 0 {
            self.apply(new, Edit::Batch { label: "Mark differences".into(), edits })?;
        }
        Ok(n)
    }
}

/// Export a PDF ▸ Word, HTML or RTF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfficeFormat {
    Docx,
    Html,
    Rtf,
}

impl OfficeFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OfficeFormat::Docx => "docx",
            OfficeFormat::Html => "html",
            OfficeFormat::Rtf => "rtf",
        }
    }

    pub fn from_extension(ext: &str) -> Option<OfficeFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "docx" => Some(OfficeFormat::Docx),
            "html" | "htm" => Some(OfficeFormat::Html),
            "rtf" => Some(OfficeFormat::Rtf),
            _ => None,
        }
    }
}

impl crate::Document {
    /// The pages as paragraphs and images (for Word, HTML and RTF export), including what
    /// form XObjects draw.
    pub fn export_pages(&self) -> Vec<pdfcraft_export::Page> {
        let Some(cos) = self.editor.as_ref().map(|e| &e.cos) else { return Vec::new() };
        self.info
            .pages
            .iter()
            .enumerate()
            .map(|(i, info)| {
                let blocks = pdfcraft_edit::reading_blocks(cos, i)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|b| !b.text.trim().is_empty())
                    .map(|b| {
                        let f = b.base_font.to_ascii_lowercase();
                        pdfcraft_export::Block {
                            text: b.text,
                            rect: b.rect,
                            size: b.size,
                            bold: f.contains("bold") || f.contains("black") || f.contains("heavy"),
                            italic: f.contains("italic") || f.contains("oblique"),
                            color: b.color,
                        }
                    })
                    .collect();
                let images = pdfcraft_edit::reading_images(cos, i)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|im| {
                        let (ext, bytes) = pdfcraft_create::image_file(cos, im.object?).ok()?;
                        Some(pdfcraft_export::Image { ext: if ext == "jpg" { "jpg" } else { "png" }, bytes, rect: im.rect })
                    })
                    .collect();
                pdfcraft_export::Page { width: info.width as f64, height: info.height as f64, blocks, images }
            })
            .collect()
    }

    /// The document as a Word, HTML or RTF file.
    pub fn export_office(&self, format: OfficeFormat) -> Vec<u8> {
        let pages = self.export_pages();
        let title = self.info.title.clone().unwrap_or_else(|| self.name.trim_end_matches(".pdf").to_string());
        match format {
            OfficeFormat::Docx => pdfcraft_export::docx(&pages, &title),
            OfficeFormat::Html => pdfcraft_export::html(&pages, &title).into_bytes(),
            OfficeFormat::Rtf => pdfcraft_export::rtf(&pages).into_bytes(),
        }
    }
}
