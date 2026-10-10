//! Changed regions for revision clouds: where two renderings of a sheet differ once lined up.
//!
//! Ink of the new version (`b`) with no ink of the old one (`a`) nearby was added; ink of the old
//! version with no new ink nearby was removed. "Nearby" allows `tolerance` pixels, which absorbs
//! anti-aliasing, a slightly different line weight and what alignment leaves over. Changed
//! pixels are counted per cell of a coarse grid; cells with enough of them, and within a couple
//! of cells of each other, make one region.

use crate::align::Mask;

/// A region where the versions differ, in pixels of `b` (`[x0, y0, x1, y1]`, y down), with how
/// many pixels of ink were added and removed there.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangedRegion {
    pub rect: [f64; 4],
    pub added: usize,
    pub removed: usize,
}

/// Grid cell size (pixels).
const CELL: usize = 8;
/// A cell counts as changed with at least this many changed pixels (fewer is noise).
const MIN_CELL: u32 = 4;
/// Changed cells this many cells apart (or closer) belong to one region.
const GAP: i64 = 2;
/// Regions with fewer changed pixels are dropped.
const MIN_REGION: usize = 12;
/// At most this many regions, the largest (a sheet that changed everywhere is better shown as
/// a few big clouds than thousands of small ones).
const MAX_REGIONS: usize = 400;

/// Where `b` differs from `a`. `map` carries a point of `b` (pixels, y down) to the matching
/// point of `a` (the alignment). Both are RGBA renderings; a pixel is ink when it is dark and
/// opaque. Regions come top to bottom, then left to right. Empty when either rendering is
/// empty or too large.
pub fn changed_regions(a: (&[u8], u32, u32), b: (&[u8], u32, u32), map: impl Fn([f64; 2]) -> [f64; 2], tolerance: u32) -> Vec<ChangedRegion> {
    let (Some(ma), Some(mb)) = (Mask::from_rgba(a), Mask::from_rgba(b)) else { return Vec::new() };
    let grow = |m: &Mask| (0..tolerance.min(8)).fold(None::<Mask>, |acc, _| Some(acc.as_ref().unwrap_or(m).dilate()));
    let (a_near, b_near) = (grow(&ma), grow(&mb));
    let (a_near, b_near) = (a_near.as_ref().unwrap_or(&ma), b_near.as_ref().unwrap_or(&mb));
    let (gw, gh) = (mb.w.div_ceil(CELL), mb.h.div_ceil(CELL));
    let mut added = vec![0u32; gw * gh];
    let mut removed = vec![0u32; gw * gh];
    for y in 0..mb.h {
        for x in 0..mb.w {
            let q = map([x as f64 + 0.5, y as f64 + 0.5]);
            if !(q[0].is_finite() && q[1].is_finite()) {
                continue;
            }
            let (qx, qy) = (q[0].floor() as i64, q[1].floor() as i64);
            let (xi, yi) = (x as i64, y as i64);
            let cell = (y / CELL) * gw + x / CELL;
            if mb.get(xi, yi)
                && !a_near.get(qx, qy)
                && let Some(c) = added.get_mut(cell)
            {
                *c += 1;
            }
            if ma.get(qx, qy)
                && !b_near.get(xi, yi)
                && let Some(c) = removed.get_mut(cell)
            {
                *c += 1;
            }
        }
    }
    let changed = |i: usize| added.get(i).copied().unwrap_or(0) + removed.get(i).copied().unwrap_or(0) >= MIN_CELL;
    // Group changed cells: a breadth-first walk joining cells up to GAP cells apart.
    let mut seen = vec![false; gw * gh];
    let mut regions = Vec::new();
    for start in 0..gw * gh {
        if !changed(start) || seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut queue = std::collections::VecDeque::from([start]);
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        let (mut plus, mut minus) = (0usize, 0usize);
        while let Some(i) = queue.pop_front() {
            let (cx, cy) = (i % gw, i / gw);
            (x0, y0, x1, y1) = (x0.min(cx), y0.min(cy), x1.max(cx), y1.max(cy));
            plus += added.get(i).copied().unwrap_or(0) as usize;
            minus += removed.get(i).copied().unwrap_or(0) as usize;
            for dy in -GAP..=GAP {
                for dx in -GAP..=GAP {
                    let (nx, ny) = (cx as i64 + dx, cy as i64 + dy);
                    if nx < 0 || ny < 0 || nx as usize >= gw || ny as usize >= gh {
                        continue;
                    }
                    let n = ny as usize * gw + nx as usize;
                    if changed(n) && !seen.get(n).copied().unwrap_or(true) {
                        if let Some(s) = seen.get_mut(n) {
                            *s = true;
                        }
                        queue.push_back(n);
                    }
                }
            }
        }
        if plus + minus >= MIN_REGION {
            let rect = [(x0 * CELL) as f64, (y0 * CELL) as f64, ((x1 + 1) * CELL).min(mb.w) as f64, ((y1 + 1) * CELL).min(mb.h) as f64];
            regions.push(ChangedRegion { rect, added: plus, removed: minus });
        }
    }
    if regions.len() > MAX_REGIONS {
        regions.sort_by_key(|r| std::cmp::Reverse(r.added + r.removed));
        regions.truncate(MAX_REGIONS);
    }
    regions.sort_by(|p, q| p.rect[1].total_cmp(&q.rect[1]).then(p.rect[0].total_cmp(&q.rect[0])));
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white `w` × `h` RGBA image with black boxes `[x0, y0, x1, y1]` (outlines `t` px thick).
    fn draw(w: u32, h: u32, boxes: &[[u32; 4]], t: u32) -> Vec<u8> {
        let mut px = vec![255u8; (w * h * 4) as usize];
        for &[x0, y0, x1, y1] in boxes {
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let edge = x < x0 + t || x + t >= x1 || y < y0 + t || y + t >= y1;
                    if edge {
                        let i = ((y * w + x) * 4) as usize;
                        px[i..i + 3].copy_from_slice(&[0, 0, 0]);
                    }
                }
            }
        }
        px
    }

    const PLAN: [[u32; 4]; 3] = [[40, 40, 560, 740], [40, 40, 300, 300], [300, 400, 560, 740]];

    #[test]
    fn finds_what_was_added_and_removed() {
        let (w, h) = (600, 800);
        let a = draw(w, h, &PLAN, 3);
        // The new version drops the second room and adds a small box elsewhere.
        let b = draw(w, h, &[PLAN[0], PLAN[2], [80, 450, 180, 520]], 3);
        let found = changed_regions((&a, w, h), (&b, w, h), |p| p, 2);
        assert_eq!(found.len(), 2, "{found:?}");
        // Top first: the removed room (its two inner walls; the shared outer walls are no change).
        let (gone, new) = (&found[0], &found[1]);
        assert!(gone.removed > 100 && gone.added == 0, "{gone:?}");
        assert!(gone.rect[0] <= 40.0 && gone.rect[2] >= 300.0 && gone.rect[3] >= 300.0 && gone.rect[1] <= 300.0, "{gone:?}");
        assert!(new.added > 100 && new.removed == 0, "{new:?}");
        assert!(new.rect[0] <= 80.0 && new.rect[2] >= 180.0 && new.rect[1] <= 450.0 && new.rect[3] >= 520.0, "{new:?}");
        assert!(new.rect[2] - new.rect[0] < 140.0, "a tight region: {new:?}");
    }

    #[test]
    fn small_misalignment_and_weight_changes_are_not_changes() {
        let (w, h) = (600, 800);
        let a = draw(w, h, &PLAN, 3);
        // One pixel off, and lines a pixel thinner.
        let b = draw(w, h, &PLAN.map(|[x0, y0, x1, y1]| [x0 + 1, y0, x1 + 1, y1]), 2);
        assert!(changed_regions((&a, w, h), (&b, w, h), |p| p, 2).is_empty());
        assert!(changed_regions((&a, w, h), (&a, w, h), |p| p, 0).is_empty());
    }

    #[test]
    fn regions_follow_the_alignment() {
        let (w, h) = (600, 800);
        let a = draw(w, h, &PLAN, 3);
        // The new sheet is the plan moved (25, −15) px, with one added box.
        let moved: Vec<[u32; 4]> = PLAN.iter().map(|&[x0, y0, x1, y1]| [x0 + 25, y0 - 15, x1 + 25, y1 - 15]).chain([[400, 100, 480, 160]]).collect();
        let b = draw(w, h, &moved, 3);
        let found = changed_regions((&a, w, h), (&b, w, h), |p| [p[0] - 25.0, p[1] + 15.0], 2);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].rect[0] <= 400.0 && found[0].rect[2] >= 480.0 && found[0].added > 100, "{found:?}");
        // Without the alignment, the whole plan reads as changed.
        assert!(changed_regions((&a, w, h), (&b, w, h), |p| p, 2).len() > 1);
    }

    #[test]
    fn junk_input_finds_nothing() {
        let a = draw(100, 100, &[[10, 10, 90, 90]], 2);
        assert!(changed_regions((&[], 0, 0), (&a, 100, 100), |p| p, 2).is_empty());
        assert!(changed_regions((&a[..40], 100, 100), (&a[..40], 100, 100), |p| p, 2).is_empty());
        assert!(changed_regions((&a, 100, 100), (&a, 100, 100), |_| [f64::NAN, 0.0], 2).is_empty());
    }
}
