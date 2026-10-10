//! Overlay Pages: compare two versions of a drawing by laying one over the other.
//!
//! Each page of the result draws page n of the old version tinted one colour (red by default)
//! and page n of the new version tinted another (blue), multiplied together. Lines both
//! versions share come out dark, lines only the old one has stay red and lines only the new one
//! has stay blue. Both versions stay vector, and each is a layer (optional content group) that
//! can be hidden to see the other alone.
//!
//! Each version is a transparency group: white paper, the page (a form XObject from
//! [`crate::page_as_form`]), a white fill with blend mode Saturation (turning it grey) and a fill
//! in its colour with blend mode Screen (black becomes the colour, white stays white). The new
//! version's group is drawn over the old one's with blend mode Multiply.

use pdfcraft_content::{Matrix, Op, num, serialize_ops};
use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString, Stream};

use crate::OrganizeError;

/// How to overlay two documents.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayOptions {
    /// The old version's colour (RGB, 0–1).
    pub old_colour: [f64; 3],
    /// The new version's colour (RGB, 0–1).
    pub new_colour: [f64; 3],
    /// Where each new page sits on the old one: a PDF matrix `[a b c d e f]` applied to the new
    /// page as displayed (points, origin at its lower-left corner). The identity lines up the
    /// lower-left corners; alignment supplies a shift, scale or rotation.
    pub new_transform: [f64; 6],
    /// The layer names.
    pub old_name: String,
    pub new_name: String,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        OverlayOptions {
            old_colour: [1.0, 0.0, 0.0],
            new_colour: [0.0, 0.0, 1.0],
            new_transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            old_name: "Old".into(),
            new_name: "New".into(),
        }
    }
}

/// A new document overlaying page n of `new` on page n of `old`, for every n. When one
/// document has more pages, its extra pages appear alone in its colour. Annotations are not
/// drawn (only page content).
pub fn overlay(old: &Document, new: &Document, opts: &OverlayOptions) -> Result<Document, OrganizeError> {
    let t = opts.new_transform;
    let det = t[0] * t[3] - t[1] * t[2];
    if t.iter().any(|v| !v.is_finite()) || !det.is_finite() || det.abs() < 1e-9 {
        return Err(OrganizeError::Invalid("the new pages' placement is not an invertible matrix".into()));
    }
    let colour = |c: [f64; 3]| c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    let (old_colour, new_colour) = (colour(opts.old_colour), colour(opts.new_colour));
    let (n_old, n_new) = (crate::page_count(old)?, crate::page_count(new)?);
    let n = n_old.max(n_new);
    if n == 0 {
        return Err(OrganizeError::Invalid("neither document has pages".into()));
    }

    let mut out = Document::new_empty();
    let blend = |out: &mut Document, mode: &str| {
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("ExtGState"));
        d.set(b"BM".to_vec(), Object::name(mode));
        out.add(d)
    };
    let modes = Blends { grey: blend(&mut out, "Saturation"), tint: blend(&mut out, "Screen") };
    let multiply = blend(&mut out, "Multiply");
    let layer = |out: &mut Document, name: &str| {
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("OCG"));
        d.set(b"Name".to_vec(), Object::String(PdfString::text(name)));
        out.add(d)
    };
    let (old_layer, new_layer) = (layer(&mut out, &opts.old_name), layer(&mut out, &opts.new_name));

    let root = crate::pages_root(&out)?;
    let mut pages = Vec::with_capacity(n);
    for i in 0..n {
        let a = if i < n_old { Some(crate::page_as_form(&mut out, old, i)?) } else { None };
        let b = if i < n_new { Some(crate::page_as_form(&mut out, new, i)?) } else { None };
        // The sheet: both pages where they land.
        let boxes = [a.map(|(_, (w, h))| [0.0, 0.0, w, h]), b.map(|(_, (w, h))| Matrix(t).bbox([0.0, 0.0, w, h]))];
        let Some(sheet) = boxes.into_iter().flatten().reduce(|p, q| [p[0].min(q[0]), p[1].min(q[1]), p[2].max(q[2]), p[3].max(q[3])]) else {
            continue;
        };
        if sheet.iter().any(|v| !v.is_finite() || v.abs() > MAX_COORD) {
            return Err(OrganizeError::Invalid(format!("page {}: the new page lands too far away to overlay", i + 1)));
        }
        let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut content = Vec::new();
        let mut xobjects = Dict::new();
        let mut props = Dict::new();
        let mut gs = Dict::new();
        if let Some((form, _)) = a {
            let g = tinted(&mut out, form, identity, sheet, old_colour, &modes);
            xobjects.set(b"Old".to_vec(), Object::Ref(g));
            props.set(b"L0".to_vec(), Object::Ref(old_layer));
            content.extend([
                Op::new("BDC", vec![Object::name("OC"), Object::name("L0")]),
                Op::new("Do", vec![Object::name("Old")]),
                Op::new("EMC", vec![]),
            ]);
        }
        if let Some((form, _)) = b {
            let g = tinted(&mut out, form, t, sheet, new_colour, &modes);
            xobjects.set(b"New".to_vec(), Object::Ref(g));
            props.set(b"L1".to_vec(), Object::Ref(new_layer));
            gs.set(b"Mu".to_vec(), Object::Ref(multiply));
            content.extend([
                Op::new("BDC", vec![Object::name("OC"), Object::name("L1")]),
                Op::new("q", vec![]),
                Op::new("gs", vec![Object::name("Mu")]),
                Op::new("Do", vec![Object::name("New")]),
                Op::new("Q", vec![]),
                Op::new("EMC", vec![]),
            ]);
        }
        let mut res = Dict::new();
        res.set(b"XObject".to_vec(), Object::Dict(xobjects));
        res.set(b"Properties".to_vec(), Object::Dict(props));
        res.set(b"ExtGState".to_vec(), Object::Dict(gs));
        let contents = out.add(Stream::flate(Dict::new(), &serialize_ops(&content)));
        let mut page = Dict::new();
        page.set(b"Type".to_vec(), Object::name("Page"));
        page.set(b"Parent".to_vec(), Object::Ref(root));
        page.set(b"MediaBox".to_vec(), Object::Array(sheet.iter().map(|v| num(*v)).collect()));
        page.set(b"Resources".to_vec(), Object::Dict(res));
        page.set(b"Contents".to_vec(), Object::Ref(contents));
        // Blending happens in RGB on this page, whatever the viewer's default.
        page.set(b"Group".to_vec(), Object::Dict(group_dict()));
        pages.push((out.add(page), Dict::new()));
    }
    crate::rebuild(&mut out, &pages)?;
    crate::import::append_layers(&mut out, &[(old_layer, true), (new_layer, true)])?;
    crate::set_info(&mut out, "Title", &format!("Overlay: {} and {}", opts.old_name, opts.new_name))?;
    Ok(out)
}

/// Alignment: the placement (a matrix for [`OverlayOptions::new_transform`]) that carries each
/// point of `new` onto the matching point of `old`. Points are on the pages as displayed, in
/// points from the lower-left corner. One pair shifts; two pairs also turn and scale evenly
/// (the usual case: the same sheet plotted again); three pairs fit any affine change, such as
/// a sheet stretched more one way than the other. Fails when the points cannot define a
/// placement (repeated or lined-up points) or would squash or blow the page up beyond reason.
pub fn alignment(old: &[[f64; 2]], new: &[[f64; 2]]) -> Result<[f64; 6], OrganizeError> {
    let bad = |why: &str| OrganizeError::Invalid(why.to_string());
    if old.len() != new.len() || old.is_empty() || old.len() > 3 {
        return Err(bad("alignment needs one, two or three points on each version, as many on one as on the other"));
    }
    if old.iter().chain(new).flatten().any(|v| !v.is_finite() || v.abs() > MAX_COORD) {
        return Err(bad("an alignment point is off the page"));
    }
    let m = match (old, new) {
        ([o], [n]) => [1.0, 0.0, 0.0, 1.0, o[0] - n[0], o[1] - n[1]],
        ([o0, o1], [n0, n1]) => {
            // As complex numbers: z = (o1 − o0) / (n1 − n0) turns and scales.
            let (ux, uy) = (n1[0] - n0[0], n1[1] - n0[1]);
            let (vx, vy) = (o1[0] - o0[0], o1[1] - o0[1]);
            let len = ux * ux + uy * uy;
            if len < MIN_SPAN * MIN_SPAN || vx * vx + vy * vy < MIN_SPAN * MIN_SPAN {
                return Err(bad("the two alignment points on a version are too close together"));
            }
            let (a, b) = ((vx * ux + vy * uy) / len, (vy * ux - vx * uy) / len);
            [a, b, -b, a, o0[0] - (a * n0[0] - b * n0[1]), o0[1] - (b * n0[0] + a * n0[1])]
        }
        ([o0, o1, o2], [n0, n1, n2]) => {
            // Solve [x y 1]·[a c e] = x' and [x y 1]·[b d f] = y' for the three pairs.
            let det = n0[0] * (n1[1] - n2[1]) - n0[1] * (n1[0] - n2[0]) + (n1[0] * n2[1] - n2[0] * n1[1]);
            // Twice the triangle's area: lined-up points leave no room to fit.
            if det.abs() < MIN_SPAN * MIN_SPAN {
                return Err(bad("the three alignment points on the new version are in a line"));
            }
            let solve = |t: [f64; 3]| -> [f64; 3] {
                let p = (t[0] * (n1[1] - n2[1]) - n0[1] * (t[1] - t[2]) + (t[1] * n2[1] - t[2] * n1[1])) / det;
                let q = (n0[0] * (t[1] - t[2]) - t[0] * (n1[0] - n2[0]) + (n1[0] * t[2] - n2[0] * t[1])) / det;
                let r =
                    (n0[0] * (n1[1] * t[2] - n2[1] * t[1]) - n0[1] * (n1[0] * t[2] - n2[0] * t[1]) + t[0] * (n1[0] * n2[1] - n2[0] * n1[1])) / det;
                [p, q, r]
            };
            let [a, c, e] = solve([o0[0], o1[0], o2[0]]);
            let [b, d, f] = solve([o0[1], o1[1], o2[1]]);
            [a, b, c, d, e, f]
        }
        _ => return Err(bad("alignment needs one, two or three points on each version")),
    };
    let det = m[0] * m[3] - m[1] * m[2];
    if m.iter().any(|v| !v.is_finite()) || !(MIN_AREA_SCALE..=1.0 / MIN_AREA_SCALE).contains(&det.abs()) {
        return Err(bad("the alignment points don't match: they would shrink or enlarge the page more than 20 times"));
    }
    Ok(m)
}

/// Alignment points closer than this (points) can't set a direction.
const MIN_SPAN: f64 = 1.0;

/// The most an alignment may scale the page's area by, either way (20× in each direction).
const MIN_AREA_SCALE: f64 = 1.0 / 400.0;

/// The furthest a sheet edge may be from the origin, in points (about 350 m): beyond any real
/// drawing, and well inside what a PDF number holds.
const MAX_COORD: f64 = 1_000_000.0;

/// The blend-mode graphics states a tinted version uses.
struct Blends {
    grey: ObjRef,
    tint: ObjRef,
}

fn group_dict() -> Dict {
    let mut g = Dict::new();
    g.set(b"Type".to_vec(), Object::name("Group"));
    g.set(b"S".to_vec(), Object::name("Transparency"));
    g.set(b"CS".to_vec(), Object::name("DeviceRGB"));
    g.set(b"I".to_vec(), Object::Bool(true));
    g
}

/// An isolated transparency group drawing `page` (placed by `m`) on white paper covering
/// `sheet`, in shades of `colour`.
fn tinted(out: &mut Document, page: ObjRef, m: [f64; 6], sheet: [f64; 4], colour: [f64; 3], modes: &Blends) -> ObjRef {
    let [x0, y0, x1, y1] = sheet;
    let rect = || Op::new("re", vec![num(x0), num(y0), num(x1 - x0), num(y1 - y0)]);
    let rgb = |c: [f64; 3]| Op::new("rg", c.iter().map(|v| num(*v)).collect());
    let ops = [
        rgb([1.0; 3]),
        rect(),
        Op::new("f", vec![]),
        Op::new("q", vec![]),
        // The page expects the initial graphics state: black, not the paper's white.
        Op::new("g", vec![num(0.0)]),
        Op::new("G", vec![num(0.0)]),
        Op::new("cm", m.iter().map(|v| num(*v)).collect()),
        Op::new("Do", vec![Object::name("P")]),
        Op::new("Q", vec![]),
        Op::new("gs", vec![Object::name("Gr")]),
        rgb([1.0; 3]),
        rect(),
        Op::new("f", vec![]),
        Op::new("gs", vec![Object::name("Tn")]),
        rgb(colour),
        rect(),
        Op::new("f", vec![]),
    ];
    let mut xo = Dict::new();
    xo.set(b"P".to_vec(), Object::Ref(page));
    let mut gs = Dict::new();
    gs.set(b"Gr".to_vec(), Object::Ref(modes.grey));
    gs.set(b"Tn".to_vec(), Object::Ref(modes.tint));
    let mut res = Dict::new();
    res.set(b"XObject".to_vec(), Object::Dict(xo));
    res.set(b"ExtGState".to_vec(), Object::Dict(gs));
    let mut d = Dict::new();
    d.set(b"Type".to_vec(), Object::name("XObject"));
    d.set(b"Subtype".to_vec(), Object::name("Form"));
    d.set(b"BBox".to_vec(), Object::Array(sheet.iter().map(|v| num(*v)).collect()));
    d.set(b"Group".to_vec(), Object::Dict(group_dict()));
    d.set(b"Resources".to_vec(), Object::Dict(res));
    out.add(Stream::flate(d, &serialize_ops(&ops)))
}
