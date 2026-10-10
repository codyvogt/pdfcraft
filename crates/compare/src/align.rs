//! Automatic alignment for Overlay Pages: the shift, scale and slight turn that best line up two
//! renderings of a sheet, found from where their ink falls.
//!
//! A coarse search on small copies lets every ink point of `b` vote for the shift that would put
//! it on each ink point of `a`, for scales from 50% to 200%; linework both versions share piles
//! its votes on the true shift, while revisions spread theirs thin. The winner is then refined
//! level by level up to full resolution by nudging the shift, scale and turn while more of `b`'s
//! ink lands on `a`'s.

/// How rendering `b` lines up with rendering `a`: a point `p` of `b` (pixels, y down) lands on
/// `scale · R(turn) · p + shift` in `a`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub scale: f64,
    /// Radians; positive turns +x towards +y (clockwise on screen).
    pub turn: f64,
    pub shift: [f64; 2],
    /// How well the versions overlap, 0–1: the smaller of the share of `b`'s ink that lands on
    /// (or next to) `a`'s ink and the share of `a`'s that lands on `b`'s.
    pub score: f64,
}

impl Fit {
    /// Where point `p` of `b` lands in `a`.
    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let (s, c) = self.turn.sin_cos();
        [self.scale * (c * p[0] - s * p[1]) + self.shift[0], self.scale * (s * p[0] + c * p[1]) + self.shift[1]]
    }

    /// Where point `p` of `a` comes from in `b` (the inverse of [`Fit::apply`]).
    pub fn invert(&self, p: [f64; 2]) -> [f64; 2] {
        let (s, c) = self.turn.sin_cos();
        let (x, y) = ((p[0] - self.shift[0]) / self.scale, (p[1] - self.shift[1]) / self.scale);
        [c * x + s * y, -s * x + c * y]
    }
}

/// Renderings larger than this (pixels per side) are refused rather than searched slowly.
const MAX_SIDE: u32 = 8192;
/// The coarse search runs on copies at most this many pixels on their longer side.
const COARSE_SIDE: usize = 160;
/// Ink points voting in the coarse search, per image.
const COARSE_POINTS: usize = 1200;
/// Ink points of `b` scored while refining.
const FINE_POINTS: usize = 20_000;
/// Scales tried by the coarse search: 0.5 × 1.04^k up to 2.
const SCALE_STEPS: i32 = 36;
const SCALE_STEP: f64 = 1.04;
/// How much better (share of ink overlapping both ways) a different scale must line up than
/// the same scale, or a large move than none, to be chosen.
const SCALE_MARGIN: f64 = 0.1;
/// Moves of a page corner up to this share of the page's longer side (half a percent: about
/// 4 pt on a letter page) are corrections, taken whenever they line up better; larger ones must
/// line up clearly better than leaving the page in place.
const MOVE_NEAR: f64 = 0.005;
/// Other scales refined in full besides the same scale.
const OTHER_SCALES: usize = 2;
/// Hill-climbing moves per pass.
const MAX_MOVES: usize = 200;

/// The fit that best lines `b` up with `a`. Both are RGBA renderings at the same resolution;
/// a pixel is ink when it is dark and opaque. `None` when either has too little ink to go on,
/// or is empty or too large.
pub fn auto_align(a: (&[u8], u32, u32), b: (&[u8], u32, u32)) -> Option<Fit> {
    let ma = Mask::from_rgba(a)?;
    let mb = Mask::from_rgba(b)?;
    // Levels: 0 is full size; each next one is half the size (an ink pixel stays ink).
    let mut levels = 0;
    while ma.w.max(ma.h) >> levels > COARSE_SIDE && levels < 6 {
        levels += 1;
    }
    let mut pa = vec![ma];
    let mut pb = vec![mb];
    for _ in 0..levels {
        let (na, nb) = (pa.last()?.half(), pb.last()?.half());
        pa.push(na);
        pb.push(nb);
    }
    let (ca, cb) = (pa.last()?, pb.last()?);
    let (va, vb) = (ca.points(COARSE_POINTS), cb.points(COARSE_POINTS));
    if va.len() < 8 || vb.len() < 8 {
        return None;
    }
    // Coarse: the best-voted shift for each scale, judged by how well the two then overlap
    // both ways (votes alone can't tell a line of text from a shrunken copy of it). The same
    // scale goes first, so it wins ties.
    let coarse = Level::new(ca, cb, COARSE_POINTS);
    let candidate = |scale: f64| {
        vote(&va, &vb, scale, ca, cb).map(|(_, shift)| {
            let mut fit = Fit { scale, turn: 0.0, shift, score: 0.0 };
            fit.score = coarse.overlap(&fit);
            fit
        })
    };
    let same = candidate(1.0);
    let mut others: Vec<Fit> = (0..=SCALE_STEPS).filter_map(|k| candidate(0.5 * SCALE_STEP.powi(k))).collect();
    others.sort_by(|x, y| y.score.total_cmp(&x.score));
    // The best other scales, not near each other (neighbouring scales find the same fit).
    let mut tops: Vec<Fit> = Vec::new();
    for f in others {
        if tops.len() < OTHER_SCALES && tops.iter().all(|t| (t.scale / f.scale).ln().abs() > SCALE_STEP.ln() * 1.5) {
            tops.push(f);
        }
    }
    // Refine each candidate from the coarse level down to full size, and judge them there.
    let finer: Vec<Level> = pa.iter().zip(&pb).map(|(a, b)| Level::new(a, b, FINE_POINTS)).collect();
    let (a0, b0) = (pa.first()?, pb.first()?);
    let distances = (a0.distances(), b0.distances());
    let refine = |fit: Fit| refine(fit, &finer, &distances);
    let same = same.map(refine);
    let other = tops.into_iter().map(refine).max_by(|x, y| x.score.total_cmp(&y.score));
    // Revisions are usually plotted at the same scale: another scale must overlap clearly
    // better to be believed (a line of text half rewritten can fit a shrunken copy of itself
    // a little better by chance).
    let fit = match (same, other) {
        (Some(s), Some(o)) if o.score < s.score + SCALE_MARGIN => s,
        (_, Some(o)) => o,
        (s, None) => s?,
    };
    // Pages already in place usually are (the same export, the same paper): moving `b` more
    // than a little must also line up clearly better than leaving it where it is, with the
    // bottom-left corners together as PDF pages are placed. (Half a rewritten line of text
    // lines up with its other half just as well a word to the side.)
    let mut home = Fit { scale: 1.0, turn: 0.0, shift: [0.0, a0.h as f64 - b0.h as f64], score: 0.0 };
    home.score = finer.first()?.overlap(&home);
    let span = b0.w.max(b0.h) as f64;
    let corners = [[0.0, 0.0], [b0.w as f64, 0.0], [0.0, b0.h as f64], [b0.w as f64, b0.h as f64]];
    let moved = corners.iter().map(|c| {
        let (p, q) = (fit.apply(*c), home.apply(*c));
        (p[0] - q[0]).hypot(p[1] - q[1])
    });
    let fit = if moved.fold(0.0, f64::max) > MOVE_NEAR * span && fit.score < home.score + SCALE_MARGIN { home } else { fit };
    fit.score.is_finite().then_some(fit)
}

/// Refine a coarse fit level by level (`levels[0]` is full size) and polish it at full size
/// with the versions' distance maps.
fn refine(mut fit: Fit, levels: &[Level], (da, db): &(Distances, Distances)) -> Fit {
    for (level, here) in levels.iter().enumerate().rev() {
        let span = here.span.max(1.0);
        fit = climb(fit, here, 1.0, 1.0 / span);
        if level == 0 {
            // Overlap is flat anywhere inside a thick line, so centre the lines on each other:
            // the average distance from each version's ink to the other's, both ways, in
            // smaller and smaller steps.
            for step in [1.0, 0.5, 0.25, 0.125] {
                fit = polish(fit, &here.b_points, da, &here.a_points, db, step, step / span);
            }
            fit.score = here.overlap(&fit);
        } else {
            // Pixel centres double from one level to the next.
            fit.shift = [fit.shift[0] * 2.0, fit.shift[1] * 2.0];
        }
    }
    fit
}

/// The shift with the most votes at `scale` (no turn), and its votes: each pair of an ink point
/// of `a` and one of `b` votes for the shift carrying the second onto the first.
fn vote(va: &[[f64; 2]], vb: &[[f64; 2]], scale: f64, a: &Mask, b: &Mask) -> Option<(u32, [f64; 2])> {
    // Shifts run from −(b's scaled size) to a's size; offset them to start at 0.
    let (ox, oy) = ((b.w as f64 * scale).ceil() as i64 + 2, (b.h as f64 * scale).ceil() as i64 + 2);
    let (aw, ah) = (a.w as i64 + ox + 3, a.h as i64 + oy + 3);
    let cells = usize::try_from(aw.checked_mul(ah)?).ok()?;
    let mut acc = vec![0u32; cells];
    for q in vb {
        let (qx, qy) = (q[0] * scale, q[1] * scale);
        for p in va {
            let (tx, ty) = ((p[0] - qx).round() as i64 + ox, (p[1] - qy).round() as i64 + oy);
            if (0..aw).contains(&tx)
                && (0..ah).contains(&ty)
                && let Some(c) = acc.get_mut((ty * aw + tx) as usize)
            {
                *c += 1;
            }
        }
    }
    // The peak of the votes summed over 3 × 3 cells (rounding splits a shift's votes).
    let at = |x: i64, y: i64| if (0..aw).contains(&x) && (0..ah).contains(&y) { acc.get((y * aw + x) as usize).copied().unwrap_or(0) } else { 0 };
    let mut best = (0u32, [0.0, 0.0]);
    for y in 0..ah {
        for x in 0..aw {
            if at(x, y) == 0 {
                continue;
            }
            let sum: u32 = (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))).map(|(dx, dy)| at(x + dx, y + dy)).sum();
            if sum > best.0 {
                best = (sum, [(x - ox) as f64, (y - oy) as f64]);
            }
        }
    }
    (best.0 > 0).then_some(best)
}

/// One level of the search: each version's ink points, and each version's ink grown by a pixel
/// for the other's points to land on.
struct Level {
    a_points: Vec<[f64; 2]>,
    b_points: Vec<[f64; 2]>,
    a_near: Mask,
    b_near: Mask,
    /// `b`'s longer side (pixels): sets the scale and turn steps.
    span: f64,
}

impl Level {
    fn new(a: &Mask, b: &Mask, points: usize) -> Level {
        Level { a_points: a.points(points), b_points: b.points(points), a_near: a.dilate(), b_near: b.dilate(), span: b.w.max(b.h) as f64 }
    }

    /// How well the versions overlap under `fit`: the smaller of the share of `b`'s ink landing
    /// on (or next to) `a`'s and the share of `a`'s landing on `b`'s.
    fn overlap(&self, fit: &Fit) -> f64 {
        let share = |points: &[[f64; 2]], target: &Mask, map: &dyn Fn([f64; 2]) -> [f64; 2]| -> f64 {
            if points.is_empty() {
                return 0.0;
            }
            let hits = points
                .iter()
                .filter(|p| {
                    let q = map(**p);
                    q[0].is_finite() && q[1].is_finite() && target.get(q[0].floor() as i64, q[1].floor() as i64)
                })
                .count();
            hits as f64 / points.len() as f64
        };
        share(&self.b_points, &self.a_near, &|p| fit.apply(p)).min(share(&self.a_points, &self.b_near, &|p| fit.invert(p)))
    }
}

/// Nudge the fit's shift (by `step` pixels), scale and turn (by `fine`, relative and radians)
/// while the versions overlap more (see [`Level::overlap`]).
fn climb(mut fit: Fit, level: &Level, step: f64, fine: f64) -> Fit {
    let mut current = level.overlap(&fit);
    for _ in 0..MAX_MOVES {
        let moves = [
            Fit { shift: [fit.shift[0] + step, fit.shift[1]], ..fit },
            Fit { shift: [fit.shift[0] - step, fit.shift[1]], ..fit },
            Fit { shift: [fit.shift[0], fit.shift[1] + step], ..fit },
            Fit { shift: [fit.shift[0], fit.shift[1] - step], ..fit },
            Fit { scale: fit.scale * (1.0 + fine), ..fit },
            Fit { scale: fit.scale * (1.0 - fine), ..fit },
            Fit { turn: fit.turn + fine, ..fit },
            Fit { turn: fit.turn - fine, ..fit },
        ];
        let Some((s, next)) = moves.iter().map(|m| (level.overlap(m), *m)).max_by(|x, y| x.0.total_cmp(&y.0)) else { break };
        if s <= current {
            break;
        }
        (current, fit) = (s, next);
    }
    fit.score = current;
    fit
}

/// Distances further than this (pixels) count as this far: a revision only one version has
/// shouldn't pull the fit around.
const DISTANCE_CAP: f32 = 8.0;

/// Nudge the fit while the symmetric distance between the versions' ink shrinks (see
/// [`chamfer`]).
fn polish(mut fit: Fit, pb: &[[f64; 2]], da: &Distances, pa: &[[f64; 2]], db: &Distances, step: f64, fine: f64) -> Fit {
    let mut current = chamfer(&fit, pb, da, pa, db);
    for _ in 0..MAX_MOVES {
        let moves = [
            Fit { shift: [fit.shift[0] + step, fit.shift[1]], ..fit },
            Fit { shift: [fit.shift[0] - step, fit.shift[1]], ..fit },
            Fit { shift: [fit.shift[0], fit.shift[1] + step], ..fit },
            Fit { shift: [fit.shift[0], fit.shift[1] - step], ..fit },
            Fit { scale: fit.scale * (1.0 + fine), ..fit },
            Fit { scale: fit.scale * (1.0 - fine), ..fit },
            Fit { turn: fit.turn + fine, ..fit },
            Fit { turn: fit.turn - fine, ..fit },
        ];
        let Some((d, next)) = moves.iter().map(|m| (chamfer(m, pb, da, pa, db), *m)).min_by(|x, y| x.0.total_cmp(&y.0)) else { break };
        if d >= current {
            break;
        }
        (current, fit) = (d, next);
    }
    fit
}

/// How far, on average, `b`'s ink lands from `a`'s under `fit`, plus how far `a`'s lands from
/// `b`'s under its inverse (pixels, each capped at [`DISTANCE_CAP`]).
fn chamfer(fit: &Fit, pb: &[[f64; 2]], da: &Distances, pa: &[[f64; 2]], db: &Distances) -> f64 {
    let mean = |pts: &[[f64; 2]], d: &Distances, map: &dyn Fn([f64; 2]) -> [f64; 2]| -> f64 {
        if pts.is_empty() {
            return f64::from(DISTANCE_CAP);
        }
        pts.iter().map(|p| f64::from(d.at(map(*p)))).sum::<f64>() / pts.len() as f64
    };
    mean(pb, da, &|p| fit.apply(p)) + mean(pa, db, &|p| fit.invert(p))
}

/// For each pixel, the distance (pixels, capped) to the nearest ink.
struct Distances {
    w: usize,
    h: usize,
    d: Vec<f32>,
}

impl Distances {
    /// Bilinear between pixel centres, so the distance changes smoothly as a point moves.
    fn at(&self, p: [f64; 2]) -> f32 {
        let (x, y) = (p[0] - 0.5, p[1] - 0.5);
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 || x >= (self.w - 1) as f64 || y >= (self.h - 1) as f64 {
            return DISTANCE_CAP;
        }
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let v = |x: usize, y: usize| self.d.get(y * self.w + x).copied().unwrap_or(DISTANCE_CAP);
        let top = v(x0, y0) * (1.0 - fx) + v(x0 + 1, y0) * fx;
        let bottom = v(x0, y0 + 1) * (1.0 - fx) + v(x0 + 1, y0 + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// Which pixels are ink.
struct Mask {
    w: usize,
    h: usize,
    ink: Vec<bool>,
}

impl Mask {
    fn from_rgba((px, w, h): (&[u8], u32, u32)) -> Option<Mask> {
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return None;
        }
        let (w, h) = (w as usize, h as usize);
        let ink = (0..w * h)
            .map(|i| match px.get(i * 4..i * 4 + 4) {
                // Dark and opaque; transparent pixels are paper.
                Some([r, g, b, a]) => *a >= 128 && (u32::from(*r) * 30 + u32::from(*g) * 59 + u32::from(*b) * 11) < 160 * 100,
                _ => false,
            })
            .collect();
        Some(Mask { w, h, ink })
    }

    fn get(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h && self.ink.get(y as usize * self.w + x as usize).copied().unwrap_or(false)
    }

    /// Half the size: a pixel is ink when any of the four it covers is.
    fn half(&self) -> Mask {
        let (w, h) = (self.w.div_ceil(2), self.h.div_ceil(2));
        let ink = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as i64 * 2, (i / w) as i64 * 2);
                self.get(x, y) || self.get(x + 1, y) || self.get(x, y + 1) || self.get(x + 1, y + 1)
            })
            .collect();
        Mask { w, h, ink }
    }

    /// Grown by a pixel each way, so near misses count.
    fn dilate(&self) -> Mask {
        let ink = (0..self.w * self.h)
            .map(|i| {
                let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
                (-1..=1).any(|dy| (-1..=1).any(|dx| self.get(x + dx, y + dy)))
            })
            .collect();
        Mask { w: self.w, h: self.h, ink }
    }

    /// The distance from each pixel to the nearest ink (a 3-4 chamfer: two passes over the
    /// image, within about 8% of the true distance), capped at [`DISTANCE_CAP`].
    fn distances(&self) -> Distances {
        let far = (DISTANCE_CAP * 3.0) as u32;
        let mut d: Vec<u32> = self.ink.iter().map(|v| if *v { 0 } else { far }).collect();
        let (w, h) = (self.w, self.h);
        let relax = |d: &mut Vec<u32>, x: usize, y: usize, nx: i64, ny: i64, cost: u32| {
            if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                let n = d.get(ny as usize * w + nx as usize).copied().unwrap_or(far);
                if let Some(c) = d.get_mut(y * w + x) {
                    *c = (*c).min(n.saturating_add(cost));
                }
            }
        };
        for y in 0..h {
            for x in 0..w {
                let (xi, yi) = (x as i64, y as i64);
                relax(&mut d, x, y, xi - 1, yi, 3);
                relax(&mut d, x, y, xi, yi - 1, 3);
                relax(&mut d, x, y, xi - 1, yi - 1, 4);
                relax(&mut d, x, y, xi + 1, yi - 1, 4);
            }
        }
        for y in (0..h).rev() {
            for x in (0..w).rev() {
                let (xi, yi) = (x as i64, y as i64);
                relax(&mut d, x, y, xi + 1, yi, 3);
                relax(&mut d, x, y, xi, yi + 1, 3);
                relax(&mut d, x, y, xi + 1, yi + 1, 4);
                relax(&mut d, x, y, xi - 1, yi + 1, 4);
            }
        }
        Distances { w, h, d: d.into_iter().map(|v| (v as f32 / 3.0).min(DISTANCE_CAP)).collect() }
    }

    /// Centres of ink pixels, evenly thinned to at most `max`.
    fn points(&self, max: usize) -> Vec<[f64; 2]> {
        let all: Vec<usize> = self.ink.iter().enumerate().filter_map(|(i, v)| v.then_some(i)).collect();
        let stride = all.len().div_ceil(max.max(1)).max(1);
        all.iter().step_by(stride).map(|i| [(i % self.w) as f64 + 0.5, (i / self.w) as f64 + 0.5]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An irregular plan (rooms, a diagonal, an arc) drawn 2 px thick into a `w` × `h` RGBA
    /// image, its coordinates passed through `place` first.
    fn plan(w: u32, h: u32, place: impl Fn([f64; 2]) -> [f64; 2], extra: bool) -> Vec<u8> {
        let mut px = vec![255u8; (w * h * 4) as usize];
        let mut line = |a: [f64; 2], b: [f64; 2]| {
            let n = ((b[0] - a[0]).hypot(b[1] - a[1]) * 2.0).ceil() as usize + 1;
            for i in 0..=n {
                let t = i as f64 / n as f64;
                let p = place([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
                for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                    let (x, y) = ((p[0] + dx - 0.5).floor(), (p[1] + dy - 0.5).floor());
                    if x >= 0.0 && y >= 0.0 && (x as u32) < w && (y as u32) < h {
                        let i = ((y as u32 * w + x as u32) * 4) as usize;
                        px[i..i + 3].copy_from_slice(&[0, 0, 0]);
                    }
                }
            }
        };
        let rect = |line: &mut dyn FnMut([f64; 2], [f64; 2]), x0: f64, y0: f64, x1: f64, y1: f64| {
            line([x0, y0], [x1, y0]);
            line([x1, y0], [x1, y1]);
            line([x1, y1], [x0, y1]);
            line([x0, y1], [x0, y0]);
        };
        rect(&mut line, 60.0, 50.0, 520.0, 700.0);
        rect(&mut line, 60.0, 50.0, 250.0, 230.0);
        rect(&mut line, 250.0, 50.0, 520.0, 330.0);
        rect(&mut line, 60.0, 430.0, 330.0, 700.0);
        rect(&mut line, 395.0, 520.0, 470.0, 610.0);
        line([330.0, 430.0], [520.0, 330.0]);
        line([90.0, 260.0], [210.0, 400.0]);
        for k in 0..24 {
            let (t0, t1) = (k as f64 * 0.11, (k + 1) as f64 * 0.11);
            line([400.0 + 60.0 * t0.cos(), 160.0 + 60.0 * t0.sin()], [400.0 + 60.0 * t1.cos(), 160.0 + 60.0 * t1.sin()]);
        }
        if extra {
            // A revision: a new room and a cross only the new version has.
            rect(&mut line, 120.0, 520.0, 260.0, 650.0);
            line([280.0, 120.0], [480.0, 300.0]);
        }
        px
    }

    #[test]
    fn finds_a_shift_scale_and_slight_turn_despite_revisions() {
        let (w, h) = (612, 792);
        let a = plan(w, h, |p| p, false);
        // New sheets: plotted 6% larger, turned 0.4° and moved (−35, 18) px; and plotted at 72%
        // (a smaller paper size) and moved (80, 40) px. Both with revisions.
        for (s, t, d) in [(1.06f64, 0.4f64.to_radians(), [-35.0, 18.0]), (0.72, 0.0, [80.0, 40.0])] {
            let place = |p: [f64; 2]| [s * (t.cos() * p[0] - t.sin() * p[1]) + d[0], s * (t.sin() * p[0] + t.cos() * p[1]) + d[1]];
            let b = plan(w, h, place, true);
            let fit = auto_align((&a, w, h), (&b, w, h)).expect("a fit");
            assert!(fit.score > 0.8, "{fit:?}");
            // The fit undoes the placement: corners of the plan come back within 1.5 px.
            for p in [[60.0, 50.0], [520.0, 700.0], [470.0, 520.0], [60.0, 700.0]] {
                let back = fit.apply(place(p));
                assert!((back[0] - p[0]).hypot(back[1] - p[1]) < 1.5, "scale {s}: {p:?} → {back:?}: {fit:?}");
            }
        }
    }

    #[test]
    fn identical_sheets_fit_exactly() {
        let a = plan(400, 500, |p| [p[0] * 0.6, p[1] * 0.6], false);
        let fit = auto_align((&a, 400, 500), (&a, 400, 500)).unwrap();
        assert!((fit.scale - 1.0).abs() < 1e-9 && fit.turn.abs() < 1e-9, "{fit:?}");
        assert!(fit.shift[0].abs() < 0.6 && fit.shift[1].abs() < 0.6, "{fit:?}");
        assert!(fit.score > 0.99, "{fit:?}");
    }

    #[test]
    fn small_offsets_are_still_corrected() {
        // A few pixels off: within what leaving the page in place would excuse, but a better fit.
        let (w, h) = (612, 792);
        let a = plan(w, h, |p| p, false);
        let b = plan(w, h, |p| [p[0] + 3.0, p[1] - 2.0], false);
        let fit = auto_align((&a, w, h), (&b, w, h)).unwrap();
        for p in [[60.0, 50.0], [520.0, 700.0]] {
            let back = fit.apply([p[0] + 3.0, p[1] - 2.0]);
            assert!((back[0] - p[0]).hypot(back[1] - p[1]) < 0.75, "{p:?} → {back:?}: {fit:?}");
        }
    }

    #[test]
    fn a_line_of_text_is_not_mistaken_for_a_shrunken_copy() {
        // Short strokes along one row, like a line of text: a scaled-down copy of it also lies
        // entirely on it, so only overlap both ways tells the true fit.
        let (w, h) = (1500u32, 2000u32);
        let mut px = vec![255u8; (w * h * 4) as usize];
        let mut x = 180u32;
        let mut k = 7u32;
        while x < 1300 {
            k = (k * 37 + 11) % 23;
            for y in 250..(268 + k % 9) {
                for dx in 0..3 {
                    let i = ((y * w + x + dx) * 4) as usize;
                    px[i..i + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
            x += 6 + k % 7;
        }
        let fit = auto_align((&px, w, h), (&px, w, h)).unwrap();
        assert!((fit.scale - 1.0).abs() < 0.002 && fit.shift[0].abs() < 1.0 && fit.shift[1].abs() < 1.0, "{fit:?}");
        assert!(fit.score > 0.95, "{fit:?}");
    }

    #[test]
    fn blank_tiny_or_huge_renderings_have_no_fit() {
        let blank = vec![255u8; 100 * 100 * 4];
        let a = plan(612, 792, |p| p, false);
        assert!(auto_align((&a, 612, 792), (&blank, 100, 100)).is_none());
        assert!(auto_align((&[], 0, 0), (&a, 612, 792)).is_none());
        // A short buffer reads as paper rather than panicking.
        assert!(auto_align((&a[..100], 612, 792), (&a, 612, 792)).is_none());
        assert!(auto_align((&a, 612, 792), (&a, 9000, 10)).is_none());
    }
}
