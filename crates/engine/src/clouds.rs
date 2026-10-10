//! Revision clouds: the areas where two versions of a document differ, found from their
//! renderings once lined up (see [`pdfcraft_compare::changed_regions`]), drawn as Cloud comments
//! on the new version or on an overlay of the two.

use std::sync::Arc;

use crate::compare::OverlayOptions;
use crate::{DocId, Edit, EditError, NewAnnotation, Session, Shape};

/// What changed in an area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// Only the new version has ink here.
    Added,
    /// Only the old version had ink here.
    Removed,
    /// Both: something was replaced or moved.
    Changed,
}

impl ChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            ChangeKind::Added => "Added",
            ChangeKind::Removed => "Removed",
            ChangeKind::Changed => "Changed",
        }
    }
}

/// An area where the new version differs from the old one.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangeRegion {
    /// Page of the new version (0-based); compared with the same page of the old one.
    pub page: usize,
    /// The area, padded, in view space of that page: points from the top-left corner of the
    /// page as displayed (`[x0, y0, x1, y1]`).
    pub rect: [f64; 4],
    /// The page's displayed height in points (to place the area on an overlay).
    pub page_height: f64,
    pub kind: ChangeKind,
}

/// Revision clouds are drawn in the compare panel's orange for visual differences.
pub const CLOUD_COLOUR: crate::Rgb = [0.94, 0.55, 0.1];

/// Clouds stand this far (points) outside the changed ink.
const CLOUD_PAD: f64 = 6.0;

/// Ink this close (pixels of the renderings, about 0.8 pt) to ink of the other version is no
/// change: anti-aliasing, line weight and what alignment leaves over.
const CHANGE_TOLERANCE: u32 = 2;

/// An area counts as added (or removed) when at least this share of its changed ink was.
const ONE_SIDED: f64 = 0.85;

impl Session {
    /// Where page n of `new` differs from page n of `old` (for every page both have), with the
    /// new pages placed on the old ones by `placement` (an Overlay Pages placement: points,
    /// y up, from the lower-left corner of each displayed page; the identity when they already
    /// line up). Areas closer than their padding are merged.
    pub fn change_regions(&self, old: DocId, new: DocId, placement: [f64; 6]) -> Result<Vec<ChangeRegion>, EditError> {
        if placement.iter().any(|v| !v.is_finite()) {
            return Err(pdfcraft_organize::OrganizeError::Invalid("the placement is not a valid matrix".into()).into());
        }
        let pages = {
            let (a, b) = (self.get(old).ok_or(EditError::NoDocument)?, self.get(new).ok_or(EditError::NoDocument)?);
            a.info.pages.len().min(b.info.pages.len())
        };
        let mut out = Vec::new();
        for page in 0..pages {
            let size = self.get(new).and_then(|d| d.info.pages.get(page)).map_or(792.0, |p| p.width.max(p.height).max(1.0));
            let scale = (crate::compare::ALIGN_SIDE / size).clamp(0.1, 4.0);
            let (a, ka, _, ha) = self.render_for_compare(old, page, scale)?;
            let (b, kb, wb, hb) = self.render_for_compare(new, page, scale)?;
            let t = placement;
            // A pixel of the new page → its point on the new page (y up) → the old page's point
            // under the placement → that pixel of the old page.
            let map = |p: [f64; 2]| {
                let (x, y) = (p[0] / kb, hb - p[1] / kb);
                let (u, v) = (t[0] * x + t[2] * y + t[4], t[1] * x + t[3] * y + t[5]);
                [u * ka, (ha - v) * ka]
            };
            let mut found: Vec<ChangeRegion> =
                pdfcraft_compare::changed_regions((&a.rgba, a.width, a.height), (&b.rgba, b.width, b.height), map, CHANGE_TOLERANCE)
                    .into_iter()
                    .map(|r| {
                        let total = (r.added + r.removed).max(1) as f64;
                        let kind = if r.added as f64 >= ONE_SIDED * total {
                            ChangeKind::Added
                        } else if r.removed as f64 >= ONE_SIDED * total {
                            ChangeKind::Removed
                        } else {
                            ChangeKind::Changed
                        };
                        let rect = [
                            (r.rect[0] / kb - CLOUD_PAD).max(0.0),
                            (r.rect[1] / kb - CLOUD_PAD).max(0.0),
                            (r.rect[2] / kb + CLOUD_PAD).min(wb),
                            (r.rect[3] / kb + CLOUD_PAD).min(hb),
                        ];
                        ChangeRegion { page, rect, page_height: hb, kind }
                    })
                    .collect();
            out.append(&mut found);
        }
        // Text changes, from the word comparison: a revision letter or one digit of a dimension
        // changes too few pixels to tell from a re-plot, but the words say exactly what changed.
        if let (Ok(text), Some(doc)) = (self.compare(old, new), self.get(new)) {
            for ch in &text.changes {
                let (rects, kind) = match ch.kind {
                    pdfcraft_compare::Kind::Replaced => (ch.new.rects.clone(), ChangeKind::Changed),
                    pdfcraft_compare::Kind::Inserted => (ch.new.rects.clone(), ChangeKind::Added),
                    // Where the words were, beside their old neighbours.
                    pdfcraft_compare::Kind::Deleted => (ch.new.near.into_iter().collect(), ChangeKind::Removed),
                };
                let page = ch.new.page;
                let Some(info) = doc.info.pages.get(page).filter(|_| page < pages) else { continue };
                let (w, h) = (f64::from(info.width), f64::from(info.height));
                for r in rects {
                    let (p, q) = (info.user_to_view(r[0] as f32, r[1] as f32), info.user_to_view(r[2] as f32, r[3] as f32));
                    let rect = [
                        (f64::from(p[0].min(q[0])) - CLOUD_PAD).max(0.0),
                        (f64::from(p[1].min(q[1])) - CLOUD_PAD).max(0.0),
                        (f64::from(p[0].max(q[0])) + CLOUD_PAD).min(w),
                        (f64::from(p[1].max(q[1])) + CLOUD_PAD).min(h),
                    ];
                    if rect.iter().all(|v| v.is_finite()) && rect[2] > rect[0] && rect[3] > rect[1] {
                        out.push(ChangeRegion { page, rect, page_height: h, kind });
                    }
                }
            }
        }
        merge_overlapping(&mut out);
        Ok(out)
    }

    /// Draw `regions` as Cloud comments (orange, authored "Compare", saying what changed) on
    /// `target`, as one undoable step; returns how many. With `placement` `None`, `target` is the
    /// new version itself; with the placement an overlay was made with, `target` is that overlay.
    pub fn add_change_clouds(&mut self, target: DocId, regions: &[ChangeRegion], placement: Option<[f64; 6]>) -> Result<usize, EditError> {
        let doc = self.get(target).ok_or(EditError::NoDocument)?;
        let mut edits = Vec::new();
        for r in regions {
            let [x0, y0, x1, y1] = r.rect;
            let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
            let vertices: Vec<[f64; 2]> = match placement {
                // The overlay's page space is the old page's, lower-left corner at the origin.
                Some(t) => corners
                    .iter()
                    .map(|[x, y]| {
                        let (u, v) = (*x, r.page_height - *y);
                        [t[0] * u + t[2] * v + t[4], t[1] * u + t[3] * v + t[5]]
                    })
                    .collect(),
                None => {
                    let Some(info) = doc.info.pages.get(r.page) else { continue };
                    corners.iter().map(|[x, y]| info.view_to_user(*x as f32, *y as f32).map(f64::from)).collect()
                }
            };
            if r.page >= doc.info.pages.len() || vertices.iter().flatten().any(|v| !v.is_finite()) {
                continue;
            }
            let shape = Shape::Polygon { vertices, cloud: true };
            let mut style = crate::Style::default_for(&shape);
            style.color = CLOUD_COLOUR;
            style.width = 1.5;
            edits.push(Edit::AddAnnotation(NewAnnotation { page: r.page, shape, style, contents: r.kind.label().into(), author: "Compare".into() }));
        }
        let n = edits.len();
        if n > 0 {
            self.apply(target, Edit::Batch { label: "Cloud changes".into(), edits })?;
        }
        Ok(n)
    }

    /// Overlay Pages with the changes clouded: the overlay (see [`Session::compare_overlay`])
    /// with a Cloud comment around each area where the versions differ once placed. Returns
    /// the new PDF (not opened) and how many clouds it has.
    pub fn compare_overlay_clouds(&mut self, old: DocId, new: DocId, opts: &OverlayOptions) -> Result<(Arc<Vec<u8>>, usize), EditError> {
        let regions = self.change_regions(old, new, opts.new_transform)?;
        let bytes = self.compare_overlay(old, new, opts)?;
        if regions.is_empty() {
            return Ok((bytes, 0));
        }
        let id = self.open("Overlay.pdf", None, bytes, None).map_err(|e| EditError::Reopen(e.to_string()))?;
        let result = self.add_change_clouds(id, &regions, Some(opts.new_transform)).and_then(|n| Ok((self.save_full_bytes(id)?, n)));
        self.close(id);
        result
    }
}

/// Merge regions on the same page whose rectangles overlap, until none do.
fn merge_overlapping(regions: &mut Vec<ChangeRegion>) {
    let overlaps = |p: &[f64; 4], q: &[f64; 4]| p[0] <= q[2] && q[0] <= p[2] && p[1] <= q[3] && q[1] <= p[3];
    loop {
        let mut merged = false;
        'outer: for i in 0..regions.len() {
            for j in i + 1..regions.len() {
                let (Some(a), Some(b)) = (regions.get(i), regions.get(j)) else { continue };
                if a.page == b.page && overlaps(&a.rect, &b.rect) {
                    let rect = [a.rect[0].min(b.rect[0]), a.rect[1].min(b.rect[1]), a.rect[2].max(b.rect[2]), a.rect[3].max(b.rect[3])];
                    let kind = if a.kind == b.kind { a.kind } else { ChangeKind::Changed };
                    let next = ChangeRegion { page: a.page, rect, page_height: a.page_height, kind };
                    regions.swap_remove(j);
                    if let Some(slot) = regions.get_mut(i) {
                        *slot = next;
                    }
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            break;
        }
    }
    regions.sort_by(|p, q| p.page.cmp(&q.page).then(p.rect[1].total_cmp(&q.rect[1])).then(p.rect[0].total_cmp(&q.rect[0])));
}
