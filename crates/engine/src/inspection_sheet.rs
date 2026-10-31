//! The setup sheet of an NC program (PDF): what the measuring technician needs at the machine.
//! A cover page with the part, the plan, the probe, an overview picture with the alignment's
//! faces and coordinate system and the steps to take; then one picture per alignment step, in
//! the setup's order, with the face highlighted and the probing points numbered, and what to do;
//! last the characteristics the program measures.

use crate::tr;
use std::collections::BTreeMap;

use serde_json::Value;
use solidcraft_geom::Vec3;
use solidcraft_render::{Camera, Rgb};

use crate::Session;
use crate::inspection::{self as insp, Frame, NcProgram, Setup};
use crate::pdf::{A4, Color, Document, Font, Page, text_width, wrap};

const NAVY: Color = Color::rgb(24, 46, 79);
const ORANGE: Color = Color::rgb(232, 116, 33);
const INK: Color = Color::rgb(33, 37, 41);
const MUTED: Color = Color::rgb(108, 117, 125);
const PANEL: Color = Color::rgb(244, 246, 249);
const RULE: Color = Color::rgb(214, 220, 228);
const WHITE: Color = Color(1.0, 1.0, 1.0);
const AXIS_X: Color = Color::rgb(214, 48, 49);
const AXIS_Y: Color = Color::rgb(39, 174, 96);
const AXIS_Z: Color = Color::rgb(41, 98, 255);
/// The highlighted face in the pictures.
const FACE: Rgb = Rgb(242, 140, 56);
/// The other faces the alignment uses (overview).
const FACE_SOFT: Rgb = Rgb(247, 190, 140);

const MARGIN: f64 = 40.0;
/// Picture width in pixels (rendered in the box's aspect, then shown at the box's size).
const PX_WIDTH: usize = 1400;

/// One alignment step: the element, its picture and its instructions.
struct Step {
    role: String,
    /// The alignment roles it plays (`primary`, `direction`, `origin_x`, `origin_y`).
    roles: Vec<String>,
    name: String,
    kind: String,
    strategy: String,
    probe: String,
    face: insp::FaceRef,
    markers: Vec<Value>,
}

/// The sheet of NC program `nc` of `setup` (whose neutral program is `program`).
pub fn setup_sheet(s: &Session, setup: &Setup, nc: &NcProgram, program: &solidcraft_inspection::program::Program) -> Result<Vec<u8>, String> {
    let model = s.world_state();
    // faces analysed as the part lies (see `inspection::in_alignment`)
    insp::in_alignment(&model, &setup.alignment, || sheet_of(s, setup, nc, program))
}

fn sheet_of(s: &Session, setup: &Setup, nc: &NcProgram, program: &solidcraft_inspection::program::Program) -> Result<Vec<u8>, String> {
    let model = s.world_state();
    let clamp = setup.alignment.clamp.is_some();
    let frame = insp::frame(&model, &setup.alignment)?;
    let design = s
        .path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).file_stem())
        .and_then(|x| x.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| tr!("Design").into());
    let setup_probe = probe_label(setup.probe.probe_file.as_deref());
    let steps: Vec<Step> = insp::alignment_elements(&model, setup)
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some(Step {
                role: e["role"].as_str()?.to_string(),
                roles: e["roles"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
                name: e["name"].as_str()?.to_string(),
                kind: e["kind"].as_str()?.to_string(),
                strategy: e["strategy"].as_str().unwrap_or_default().to_string(),
                probe: e["probe_file"].as_str().map_or_else(|| setup_probe.clone(), |p| probe_label(Some(p))),
                face: serde_json::from_value(e["face"].clone()).ok()?,
                markers: e["markers"].as_array().cloned().unwrap_or_default(),
            })
        })
        .collect();
    if steps.is_empty() {
        return Err(tr!("the alignment has no elements yet").into());
    }
    let bbox = model_box(&model);
    let post = insp::post_path(nc).map_or_else(|| "–".to_string(), |p| insp::post_name(&p));
    let sorted = !nc.options.get("sort_by_probe").is_some_and(|v| matches!(v.trim(), "off" | "false" | "0"));
    let title = tr!("Setup sheet {plan}", plan = program.plan);
    let mut doc = Document::new(&title);
    let date = today();
    let safety = setup.safety_plane.label().map_or_else(|| tr!("automatic (the face's direction)").to_string(), str::to_string);

    // ---------------------------------------------------------------- cover
    let page = doc.pages();
    {
        let p = doc.page();
        band(p, &program.plan, &format!("{design} · {} · {}", setup.name, nc.name));
    }
    let mut y = A4.1 - 132.0;
    {
        let p = doc.page_mut(page).ok_or(tr!("page missing"))?;
        let rows: Vec<(&str, String)> = vec![
            (tr!("Design"), design.clone()),
            (tr!("Setup"), setup.name.clone()),
            (tr!("NC program"), nc.name.clone()),
            (tr!("Postprocessor"), post.clone()),
            (tr!("Inspection plan"), program.plan.clone()),
            (tr!("Probe (setup)"), setup_probe.clone()),
            (tr!("Safety plane"), safety.clone()),
            (tr!("Run order"), if sorted { tr!("sorted by probe").into() } else { tr!("as in the tree").into() }),
            (tr!("Created"), date.clone()),
        ];
        y = info_grid(p, MARGIN, y, A4.0 - 2.0 * MARGIN, &rows);
    }
    // overview picture: every alignment face, numbered, and the coordinate system
    // the overview shrinks a little when there are many steps (the table must fit above the footer)
    let (iw, ih) = (A4.0 - 2.0 * MARGIN, (250.0 - (steps.len() as f64 - 3.0).max(0.0) * 30.0).clamp(140.0, 250.0));
    let iy = y - 14.0 - ih;
    let overview_cam = camera_for(&bbox, Vec3::new(-0.62, -0.78, 0.62), (iw - 12.0) / (ih - 12.0));
    let faces: Vec<(&insp::FaceRef, bool)> = steps.iter().map(|st| (&st.face, false)).collect();
    let (jpeg, w, h, scene_radius) = picture(s, &overview_cam, &faces, (iw - 12.0) / (ih - 12.0))?;
    {
        let p = doc.page_mut(page).ok_or(tr!("page missing"))?;
        p.round_rect(MARGIN, iy, iw, ih, 8.0, Some(PANEL), None);
    }
    let (bw, bh) = fit_box(w, h, iw - 12.0, ih - 12.0);
    let (bx, by) = (MARGIN + (iw - bw) / 2.0, iy + (ih - bh) / 2.0);
    doc.image(page, jpeg, w as u32, h as u32, bx, by, bw, bh);
    {
        let p = doc.page_mut(page).ok_or(tr!("page missing"))?;
        let to = Projector { cam: overview_cam, w: w as f64, h: h as f64, radius: scene_radius, x: bx, y: by, bw, bh };
        draw_frame(p, &to, &frame, bbox.diagonal() * 0.18);
        for (i, st) in steps.iter().enumerate() {
            if let Some((x, y)) = to.at(anchor(st)) {
                badge(p, x, y, &(i + 1).to_string(), 9.0);
            }
        }
        p.round_rect(MARGIN, iy, iw, ih, 8.0, None, Some((RULE, 0.8)));
        p.text(
            MARGIN + 2.0,
            iy - 11.0,
            tr!("Overview: alignment faces (numbered like the steps) and the plan's coordinate system"),
            Font::Regular,
            8.0,
            MUTED,
        );
        y = iy - 34.0;
        p.text(MARGIN, y, tr!("At the machine"), Font::Bold, 13.0, NAVY);
        y -= 18.0;
        let mut todo = vec![
            if clamp {
                tr!("Clamp the part as in the picture: it stands with the face of step 1 on the clamping plate; Z points up, into the part.")
                    .to_string()
            } else {
                tr!("Clamp the part as in the picture: the primary plane (step 1) faces up (+Z of the machine).").to_string()
            },
            tr!("Load the probe: {probe}{others}.", probe = setup_probe, others = other_probes(&steps, &setup_probe)),
            tr!(
                "Open the inspection plan “{plan}” in the measuring software (output: {folder}).",
                plan = program.plan,
                folder = insp::output_dir(nc).map_or_else(|| "–".into(), |d| d.display().to_string())
            ),
            tr!("Probe the base alignment by hand, in the order of the following pages, points as in the picture.").to_string(),
            tr!("Start the CNC run; between elements the probe retracts to the safety plane {safety}.", safety = safety),
            tr!("Check the report after the run.").to_string(),
        ];
        if !nc.no_step {
            todo.insert(
                3,
                tr!(
                    "Import the CAD model “{plan}.step” (same folder) into the plan. It already lies in the base system: do not move or realign it.",
                    plan = program.plan
                ),
            );
        }
        for (i, t) in todo.iter().enumerate() {
            badge(p, MARGIN + 8.0, y + 3.0, &(i + 1).to_string(), 7.5);
            y = p.paragraph(MARGIN + 22.0, y, A4.0 - 2.0 * MARGIN - 22.0, t, Font::Regular, 9.5, 12.5, INK) - 4.0;
        }
        y -= 6.0;
        // the steps at a glance
        p.text(MARGIN, y, tr!("Alignment"), Font::Bold, 13.0, NAVY);
        y -= 8.0;
        let cols = [(tr!("No."), 30.0), (tr!("Task"), 130.0), (tr!("Element"), 150.0), (tr!("Probing (CNC)"), 130.0), (tr!("Probe"), 75.0)];
        y = table(
            p,
            MARGIN,
            y,
            &cols,
            &steps
                .iter()
                .enumerate()
                .map(|(i, st)| vec![(i + 1).to_string(), st.role.clone(), st.name.clone(), st.strategy.clone(), st.probe.clone()])
                .collect::<Vec<_>>(),
        );
        let _ = y;
    }

    // ---------------------------------------------------------------- one block per step
    let block_h = (A4.1 - 2.0 * MARGIN - 70.0) / 2.0;
    for (i, st) in steps.iter().enumerate() {
        let top_of_page = i % 2 == 0;
        if top_of_page {
            let p = doc.page();
            small_band(p, &program.plan, tr!("Alignment – step by step"));
        }
        let page = doc.pages() - 1;
        let top = if top_of_page { A4.1 - 70.0 } else { A4.1 - 70.0 - block_h - 8.0 };
        // the picture: this face highlighted, from where it is seen best
        let normal = face_normal(st);
        let back = view_from(&st.kind, normal);
        let (pw, ph) = (322.0, block_h - 56.0);
        let cam = camera_for(&focus_box(st, &bbox), back, (pw - 10.0) / (ph - 10.0));
        let (jpeg, w, h, radius) = picture(s, &cam, &[(&st.face, true)], (pw - 10.0) / (ph - 10.0))?;
        let (px, py) = (MARGIN, top - 44.0 - ph);
        {
            let p = doc.page_mut(page).ok_or(tr!("page missing"))?;
            // header: number, role, element
            badge(p, MARGIN + 13.0, top - 14.0, &(i + 1).to_string(), 13.0);
            p.text(MARGIN + 34.0, top - 12.0, &tr!("Step {n} · {role}", n = i + 1, role = st.role), Font::Bold, 14.0, NAVY);
            p.text(MARGIN + 34.0, top - 27.0, &format!("{} – {}", st.name, purpose(&st.roles)), Font::Regular, 9.5, MUTED);
            p.round_rect(px, py, pw, ph, 8.0, Some(PANEL), None);
        }
        let (bw, bh) = fit_box(w, h, pw - 10.0, ph - 10.0);
        let (bx, by) = (px + (pw - bw) / 2.0, py + (ph - bh) / 2.0);
        doc.image(page, jpeg, w as u32, h as u32, bx, by, bw, bh);
        let p = doc.page_mut(page).ok_or(tr!("page missing"))?;
        let to = Projector { cam, w: w as f64, h: h as f64, radius, x: bx, y: by, bw, bh };
        draw_frame(p, &to, &frame, bbox.diagonal() * 0.12);
        let count = draw_markers(p, &to, &st.markers, bbox.diagonal() * 0.11);
        p.round_rect(px, py, pw, ph, 8.0, None, Some((RULE, 0.8)));
        // the instructions beside it
        let (cx, cw) = (px + pw + 14.0, A4.0 - MARGIN - (px + pw + 14.0));
        let mut cy = top - 52.0;
        let facts: Vec<(&str, String)> = vec![
            (tr!("Element"), st.name.clone()),
            (tr!("Probing by hand"), manual(&st.kind, &st.roles, clamp)),
            (tr!("CNC strategy"), st.strategy.clone()),
            (tr!("Points in the picture"), if count > 0 { count.to_string() } else { tr!("paths").into() }),
            (tr!("Probe"), st.probe.clone()),
        ];
        for (k, v) in &facts {
            p.text(cx, cy, k, Font::Regular, 7.5, MUTED);
            cy -= 11.0;
            cy = p.paragraph(cx, cy, cw, v, Font::Bold, 9.5, 11.5, INK) - 5.0;
        }
        cy -= 2.0;
        p.round_rect(cx - 6.0, py, cw + 6.0, cy - py + 2.0, 6.0, None, None);
        p.line(cx, cy + 6.0, cx + cw, cy + 6.0, RULE, 0.6);
        cy -= 8.0;
        p.text(cx, cy, tr!("How to"), Font::Bold, 10.0, ORANGE);
        cy -= 13.0;
        let _ = p.paragraph(cx, cy, cw, &instruction(&st.kind, &st.roles, &st.name, clamp), Font::Regular, 9.0, 11.5, INK);
        if top_of_page && i + 1 == steps.len() {
            notes(p, top - block_h - 8.0, block_h - 20.0);
        }
        if !top_of_page || i + 1 == steps.len() {
            continue;
        }
        p.line(MARGIN, top - block_h - 4.0, A4.0 - MARGIN, top - block_h - 4.0, RULE, 0.6);
    }

    // ---------------------------------------------------------------- characteristics
    let chars: Vec<Vec<String>> = setup
        .characteristics
        .iter()
        .filter(|c| nc.outputs(c.id))
        .map(|c| {
            let info = insp::characteristic_info(&model, Some(&frame), c);
            vec![
                info["name"].as_str().unwrap_or_default().to_string(),
                insp::kind_label(&c.kind).to_string(),
                info["nominal"].as_str().map_or_else(|| format!("⚠ {}", info["error"].as_str().unwrap_or_default()), str::to_string),
                probe_label(c.probe_file.as_deref().or(setup.probe.probe_file.as_deref())),
            ]
        })
        .collect();
    if !chars.is_empty() {
        let mut first = true;
        let mut rest: &[Vec<String>] = &chars;
        while !rest.is_empty() {
            let p = doc.page();
            small_band(p, &program.plan, if first { tr!("Characteristics in this program") } else { tr!("Characteristics (continued)") });
            first = false;
            let fit = 42.min(rest.len());
            let (now, later) = rest.split_at(fit);
            let cols = [(tr!("Characteristic"), 175.0), (tr!("Kind"), 85.0), (tr!("Nominal / tolerance"), 180.0), (tr!("Probe"), 75.0)];
            let _ = table(p, MARGIN, A4.1 - 84.0, &cols, now);
            rest = later;
        }
    }

    // ---------------------------------------------------------------- footers
    let total = doc.pages();
    for i in 0..total {
        if let Some(p) = doc.page_mut(i) {
            p.line(MARGIN, 34.0, A4.0 - MARGIN, 34.0, RULE, 0.6);
            p.text(MARGIN, 22.0, &format!("{} · {} · {}", program.plan, nc.name, date), Font::Regular, 7.5, MUTED);
            p.text_right(A4.0 - MARGIN, 22.0, &tr!("Page {n} of {total}", n = i + 1, total = total), Font::Regular, 7.5, MUTED);
        }
    }
    Ok(doc.finish())
}

/// Maps model points into the picture box on the page.
struct Projector {
    cam: Camera,
    w: f64,
    h: f64,
    radius: f64,
    x: f64,
    y: f64,
    bw: f64,
    bh: f64,
}

impl Projector {
    fn at(&self, p: Vec3) -> Option<(f64, f64)> {
        let (sx, sy, _) = self.cam.to_screen(p, self.w, self.h, self.radius)?;
        let (x, y) = (self.x + sx / self.w * self.bw, self.y + self.bh - sy / self.h * self.bh);
        (x >= self.x - 2.0 && x <= self.x + self.bw + 2.0 && y >= self.y - 2.0 && y <= self.y + self.bh + 2.0).then_some((x, y))
    }
}

fn v(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn arr(v: &Value) -> Option<[f64; 3]> {
    Some([v.get(0)?.as_f64()?, v.get(1)?.as_f64()?, v.get(2)?.as_f64()?])
}

/// The bounding box of all bodies.
fn model_box(model: &solidcraft_doc::ModelState) -> solidcraft_geom::Aabb3 {
    let mut b = solidcraft_geom::Aabb3::default();
    for body in &model.bodies {
        b = b.union(&body.mesh().bounds());
    }
    b
}

/// A camera looking from `back` (towards the eye) at the whole part, framed so the part fills
/// most of a picture of `aspect` (width / height).
fn camera_for(bbox: &solidcraft_geom::Aabb3, back: Vec3, aspect: f64) -> Camera {
    let mut cam = Camera::default().looking_from(back);
    cam.fit(bbox);
    if bbox.is_empty() {
        return cam;
    }
    // the box's corners across the view (the camera is orthographic: no perspective)
    let (right, up, _) = cam.basis();
    let (lo, hi) = (bbox.min, bbox.max);
    let mut u = (f64::INFINITY, f64::NEG_INFINITY);
    let mut w = (f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..8 {
        let c = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
        let d = c - cam.target;
        let (cu, cw) = (d.dot(right), d.dot(up));
        u = (u.0.min(cu), u.1.max(cu));
        w = (w.0.min(cw), w.1.max(cw));
    }
    cam.target = cam.target + right * ((u.0 + u.1) / 2.0) + up * ((w.0 + w.1) / 2.0);
    let half = ((w.1 - w.0) / 2.0).max((u.1 - u.0) / (2.0 * aspect.max(0.1))) / 0.86;
    if half.is_finite() && half > 0.0 && cam.fov == 0.0 {
        cam.distance = half / 0.4;
    }
    cam
}

/// Renders the part from `cam` with `faces` coloured (true: the face of this step), as a JPEG;
/// returns it, its size and the scene radius the projection needs.
fn picture(s: &Session, cam: &Camera, faces: &[(&insp::FaceRef, bool)], aspect: f64) -> Result<(Vec<u8>, usize, usize, f64), String> {
    let model = s.world_state();
    let guides = crate::view::Guides { grid: false, axes: false };
    let mut scene = crate::view::scene_with_guides(s, cam, None, guides);
    scene.background = Some((Rgb(250, 251, 253), Rgb(232, 236, 242)));
    // colour the faces: every B-rep face of their surface patch (both halves of a cylinder)
    let mut by_body: BTreeMap<String, Vec<(u32, Rgb)>> = BTreeMap::new();
    for (f, main) in faces {
        let Some(body) = model.body(&f.body) else { continue };
        let Ok(info) = insp::analyze(&model, f) else { continue };
        let mesh = body.mesh();
        let colour = if *main { FACE } else { FACE_SOFT };
        for face in mesh.surface_patch(info.index) {
            if let Ok(i) = u32::try_from(face) {
                by_body.entry(f.body.clone()).or_default().push((i, colour));
            }
        }
    }
    for body in &model.bodies {
        let Some(colours) = by_body.get(&body.name) else { continue };
        let mesh = body.mesh();
        for sm in scene.meshes.iter_mut().filter(|m| std::sync::Arc::ptr_eq(&m.mesh, &mesh)) {
            sm.face_colors.extend(colours.iter().copied());
        }
    }
    let h = ((PX_WIDTH as f64 / aspect.clamp(0.5, 3.0)).round() as usize).clamp(200, 2000);
    let canvas = solidcraft_render::render(&scene, cam, PX_WIDTH, h);
    let (w, h) = (canvas.w, canvas.h);
    let rgb: Vec<u8> = canvas.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let img = image::RgbImage::from_raw(u32::try_from(w).map_err(|e| e.to_string())?, u32::try_from(h).map_err(|e| e.to_string())?, rgb)
        .ok_or(tr!("the picture could not be made"))?;
    let mut jpeg = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 88);
    enc.encode_image(&img).map_err(|e| tr!("Picture: {e}", e = e))?;
    Ok((jpeg, w, h, scene.radius))
}

/// The largest box of the image's aspect inside (bw, bh).
fn fit_box(w: usize, h: usize, bw: f64, bh: f64) -> (f64, f64) {
    let a = w as f64 / (h as f64).max(1.0);
    if bw / bh > a { (bh * a, bh) } else { (bw, bw / a) }
}

/// What a step's picture frames: its probing points and face with room around them (a small
/// face fills the picture, a large one shows most of the part).
fn focus_box(st: &Step, part: &solidcraft_geom::Aabb3) -> solidcraft_geom::Aabb3 {
    let mut b = solidcraft_geom::Aabb3::default();
    b.add(v(st.face.point));
    for m in &st.markers {
        if let Some(p) = arr(&m["at"]) {
            b.add(v(p));
        }
        if let (Some(c), Some(r)) = (arr(&m["center"]), m["radius"].as_f64()) {
            let c = v(c);
            for d in [Vec3::X, Vec3::Y, Vec3::Z] {
                b.add(c + d * r);
                b.add(c - d * r);
            }
        }
    }
    let grow = (b.diagonal() * 0.7).max(part.diagonal() * 0.15);
    let g = Vec3::new(grow, grow, grow) * 0.5;
    let mut out = solidcraft_geom::Aabb3::default();
    out.add(b.min - g);
    out.add(b.max + g);
    out
}

/// Where a step's number goes in the overview: the middle of its probing points (of a path's
/// centre), else the point picked on the face.
fn anchor(st: &Step) -> Vec3 {
    let pts: Vec<Vec3> = st.markers.iter().filter_map(|m| arr(&m["at"]).or_else(|| arr(&m["center"]))).map(v).collect();
    if pts.is_empty() {
        return v(st.face.point);
    }
    pts.iter().fold(Vec3::ZERO, |a, p| a + *p) * (1.0 / pts.len() as f64)
}

/// The outward normal of a step's face (the first marker's, a plane's), else up.
fn face_normal(st: &Step) -> Vec3 {
    st.markers
        .iter()
        .find_map(|m| if m["type"] == "point" && st.kind == "plane" { arr(&m["normal"]) } else { None })
        .map(v)
        .and_then(|n| n.normalized())
        .unwrap_or(Vec3::Z)
}

/// Where to look at a face from: in front of a side face and a bit above, onto a top face at
/// an angle, into a bore from above.
fn view_from(kind: &str, n: Vec3) -> Vec3 {
    let iso = Vec3::new(-0.62, -0.78, 0.62);
    if kind != "plane" {
        return (Vec3::Z * 1.6 + iso).normalized().unwrap_or(Vec3::Z);
    }
    if n.z.abs() > 0.9 {
        return (iso + Vec3::Z * 0.6 * n.z.signum()).normalized().unwrap_or(Vec3::Z);
    }
    let side = Vec3::Z.cross(n).normalized().unwrap_or(Vec3::X);
    (n * 1.0 + Vec3::Z * 0.55 + side * 0.35).normalized().unwrap_or(n)
}

/// The part coordinate system: arrows X, Y, Z from its origin.
fn draw_frame(p: &mut Page, to: &Projector, fr: &Frame, len: f64) {
    let o = v(fr.origin);
    let Some((ox, oy)) = to.at(o) else { return };
    for (axis, c, label) in [(fr.x, AXIS_X, "X"), (fr.y, AXIS_Y, "Y"), (fr.z, AXIS_Z, "Z")] {
        if let Some((x, y)) = to.at(o + v(axis) * len) {
            p.arrow(ox, oy, x, y, c, 1.6, 7.0);
            let (dx, dy) = (x - ox, y - oy);
            let l = dx.hypot(dy).max(1.0);
            p.text_center(x + dx / l * 8.0, y + dy / l * 8.0 - 3.0, label, Font::Bold, 9.0, c);
        }
    }
    p.circle(ox, oy, 2.6, Some(INK), Some((WHITE, 0.8)));
}

/// The probing points (numbered, with the probing direction) and paths of a step; returns how
/// many points it drew.
fn draw_markers(p: &mut Page, to: &Projector, markers: &[Value], len: f64) -> usize {
    let mut n = 0;
    for m in markers {
        match m["type"].as_str() {
            Some("point") => {
                let (Some(at), Some(normal)) = (arr(&m["at"]), arr(&m["normal"])) else { continue };
                let (at, normal) = (v(at), v(normal).normalized().unwrap_or(Vec3::Z));
                let Some((x, y)) = to.at(at) else { continue };
                n += 1;
                if let Some((sx, sy)) = to.at(at + normal * len) {
                    p.arrow(sx, sy, x, y, NAVY, 1.5, 6.5);
                }
                p.circle(x, y, 4.2, Some(ORANGE), Some((WHITE, 1.0)));
                let label = n.to_string();
                let w = text_width(&label, Font::Bold, 8.0) + 6.0;
                p.round_rect(x + 5.0, y + 3.0, w, 11.0, 3.0, Some(NAVY), None);
                p.text(x + 8.0, y + 5.6, &label, Font::Bold, 8.0, WHITE);
            }
            Some("circle") => {
                let (Some(c), Some(normal), Some(r)) = (arr(&m["center"]), arr(&m["normal"]), m["radius"].as_f64()) else { continue };
                let (c, nz) = (v(c), v(normal).normalized().unwrap_or(Vec3::Z));
                let a = if nz.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
                let u = nz.cross(a).normalized().unwrap_or(Vec3::X);
                let w = nz.cross(u);
                let pts: Vec<(f64, f64)> = (0..=48)
                    .filter_map(|k| {
                        let t = k as f64 / 48.0 * std::f64::consts::TAU;
                        to.at(c + u * (r * t.cos()) + w * (r * t.sin()))
                    })
                    .collect();
                p.polyline(&pts, false, ORANGE, 1.6);
            }
            _ => {}
        }
    }
    n
}

/// The navy band at the top of the cover.
fn band(p: &mut Page, plan: &str, sub: &str) {
    p.rect(0.0, A4.1 - 104.0, A4.0, 104.0, NAVY);
    p.rect(0.0, A4.1 - 108.0, A4.0, 4.0, ORANGE);
    p.text(MARGIN, A4.1 - 36.0, tr!("SETUP SHEET · COORDINATE MEASURING"), Font::Bold, 9.0, Color::rgb(170, 190, 220));
    let mut size = 24.0;
    while text_width(plan, Font::Bold, size) > A4.0 - 2.0 * MARGIN && size > 12.0 {
        size -= 1.0;
    }
    p.text(MARGIN, A4.1 - 66.0, plan, Font::Bold, size, WHITE);
    p.text(MARGIN, A4.1 - 86.0, sub, Font::Regular, 11.0, Color::rgb(210, 220, 235));
}

/// The thin band of the following pages.
fn small_band(p: &mut Page, plan: &str, what: &str) {
    p.rect(0.0, A4.1 - 46.0, A4.0, 46.0, NAVY);
    p.rect(0.0, A4.1 - 49.0, A4.0, 3.0, ORANGE);
    p.text(MARGIN, A4.1 - 29.0, what, Font::Bold, 13.0, WHITE);
    p.text_right(A4.0 - MARGIN, A4.1 - 29.0, plan, Font::Regular, 9.0, Color::rgb(190, 205, 228));
}

/// A number in an orange disc.
fn badge(p: &mut Page, x: f64, y: f64, label: &str, r: f64) {
    p.circle(x, y, r, Some(ORANGE), Some((WHITE, 1.2)));
    let size = r * 1.05;
    p.text_center(x, y - size * 0.36, label, Font::Bold, size, WHITE);
}

/// Label/value pairs in two columns on a panel; returns the y below it.
fn info_grid(p: &mut Page, x: f64, top: f64, w: f64, rows: &[(&str, String)]) -> f64 {
    let per_col = rows.len().div_ceil(2);
    let h = per_col as f64 * 26.0 + 14.0;
    p.round_rect(x, top - h, w, h, 8.0, Some(PANEL), None);
    let cw = w / 2.0;
    for (i, (k, val)) in rows.iter().enumerate() {
        let (col, row) = (i / per_col, i % per_col);
        let (cx, cy) = (x + 14.0 + col as f64 * cw, top - 20.0 - row as f64 * 26.0);
        p.text(cx, cy + 4.0, k, Font::Regular, 7.5, MUTED);
        let mut text = val.clone();
        while text_width(&text, Font::Bold, 10.0) > cw - 24.0 && text.chars().count() > 4 {
            text = format!("{}…", text.chars().take(text.chars().count().saturating_sub(2)).collect::<String>());
        }
        p.text(cx, cy - 8.0, &text, Font::Bold, 10.0, INK);
    }
    top - h
}

/// A table with a header row; long cells wrap. Returns the y below it.
fn table(p: &mut Page, x: f64, top: f64, cols: &[(&str, f64)], rows: &[Vec<String>]) -> f64 {
    let width: f64 = cols.iter().map(|c| c.1).sum();
    let mut y = top - 4.0;
    p.rect(x, y - 16.0, width, 16.0, NAVY);
    let mut cx = x;
    for (name, w) in cols {
        p.text(cx + 5.0, y - 11.5, name, Font::Bold, 8.0, WHITE);
        cx += w;
    }
    y -= 16.0;
    for (i, row) in rows.iter().enumerate() {
        let lines: Vec<Vec<String>> = cols.iter().zip(row).map(|((_, w), c)| wrap(c, Font::Regular, 8.5, w - 10.0)).collect();
        let n = lines.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let h = n as f64 * 10.5 + 7.0;
        if i % 2 == 1 {
            p.rect(x, y - h, width, h, PANEL);
        }
        let mut cx = x;
        for ((_, w), cell) in cols.iter().zip(&lines) {
            let mut ly = y - 11.0;
            for l in cell {
                p.text(cx + 5.0, ly, l, if cx == x { Font::Bold } else { Font::Regular }, 8.5, INK);
                ly -= 10.5;
            }
            cx += w;
        }
        y -= h;
        p.line(x, y, x + width, y, RULE, 0.5);
    }
    y
}

/// Room for the technician's notes and sign-off.
fn notes(p: &mut Page, top: f64, h: f64) {
    let h = h.max(120.0);
    p.round_rect(MARGIN, top - h, A4.0 - 2.0 * MARGIN, h, 8.0, None, Some((RULE, 0.8)));
    p.text(MARGIN + 14.0, top - 22.0, tr!("Remarks"), Font::Bold, 11.0, NAVY);
    let mut y = top - 46.0;
    while y > top - h + 60.0 {
        p.line(MARGIN + 14.0, y, A4.0 - MARGIN - 14.0, y, RULE, 0.6);
        y -= 22.0;
    }
    let base = top - h + 22.0;
    let w = (A4.0 - 2.0 * MARGIN - 28.0 - 24.0) / 3.0;
    for (i, label) in [tr!("Date"), tr!("Operator"), tr!("Approval")].iter().enumerate() {
        let x = MARGIN + 14.0 + i as f64 * (w + 12.0);
        p.line(x, base + 10.0, x + w, base + 10.0, MUTED, 0.6);
        p.text(x, base, label, Font::Regular, 7.5, MUTED);
    }
}

/// What an alignment step fixes (by its roles: `primary`, `direction`, `origin_x`, `origin_y`).
fn purpose(roles: &[String]) -> String {
    let parts: Vec<&str> = roles
        .iter()
        .map(|r| match r.as_str() {
            "primary" => tr!("spatial axis Z and origin Z"),
            "direction" => tr!("rotation about Z (direction of the X axis)"),
            other => insp::role_label(other),
        })
        .collect();
    tr!("fixes: {what}", what = parts.join(", "))
}

/// The origins a step fixes, for the instructions ("origin X and origin Y").
fn origins(roles: &[String]) -> String {
    roles.iter().filter(|r| r.starts_with("origin_")).map(|r| insp::role_label(r)).collect::<Vec<_>>().join(tr!(" and "))
}

/// How many points to probe by hand for the manual alignment.
fn manual(kind: &str, roles: &[String], clamp: bool) -> String {
    let has = |r: &str| roles.iter().any(|x| x == r);
    match kind {
        "plane" if has("primary") && clamp => tr!("at least 3 points on the plate, around the part").into(),
        "plane" if has("primary") => tr!("at least 3 points, spread wide").into(),
        "plane" if has("direction") => tr!("at least 2 points, far apart").into(),
        "plane" => tr!("at least 1 point (better 2)").into(),
        _ => tr!("at least 4 points around the circumference").into(),
    }
}

/// The instruction text of a step.
fn instruction(kind: &str, roles: &[String], name: &str, clamp: bool) -> String {
    let has = |r: &str| roles.iter().any(|x| x == r);
    let origin = origins(roles);
    match kind {
        "plane" if has("primary") && clamp => tr!(
            "The part stands on the clamping plate with this face. Probe the plate beside the part from above: at least 3 points around it, as far apart as possible. The numbered points in the picture are the ones the CNC run takes later."
        )
        .to_string(),
        "plane" if has("primary") => tr!(
            "Probe the orange face ({name}) from above: at least 3 points, as far apart as possible and not on one line. The numbered points in the picture are the ones the CNC run takes later: probe close to them.",
            name = name
        ),
        "plane" if has("direction") => {
            let also = if origin.is_empty() { String::new() } else { tr!(" The same face also fixes the {origin}.", origin = origin) };
            tr!(
                "Probe the orange side face ({name}) square to the face, as the arrows show: at least 2 points at the same height, as far apart as possible. The line through the points fixes the part's rotation about Z.{also}",
                name = name,
                also = also
            )
        }
        "plane" => tr!(
            "Probe the orange side face ({name}) square to the face, as the arrows show. It fixes the {origin}; one point is enough, two are safer.",
            name = name,
            origin = if origin.is_empty() { tr!("origin").to_string() } else { origin }
        ),
        "circle" => tr!(
            "Move into the orange bore ({name}) and probe at least 4 points evenly around it just below the top edge. Its centre becomes the {origin}.",
            name = name,
            origin = if origin.is_empty() { tr!("reference point").to_string() } else { origin }
        ),
        _ => tr!(
            "Move into the orange bore ({name}) and probe at least 4 points evenly around it, about 2 mm below the top edge. The CNC run then measures the paths drawn in; where the axis meets the primary plane lies the {origin}.",
            name = name,
            origin = if origin.is_empty() { tr!("reference point").to_string() } else { origin }
        ),
    }
}

/// A probe's name for people: the probe table name, or the post's standard probe.
fn probe_label(p: Option<&str>) -> String {
    p.map_or_else(|| tr!("post default").to_string(), insp::probe_key)
}

/// The probes the steps use besides the setup's.
fn other_probes(steps: &[Step], setup: &str) -> String {
    let mut v: Vec<&str> = steps.iter().map(|s| s.probe.as_str()).filter(|p| *p != setup).collect();
    v.sort_unstable();
    v.dedup();
    if v.is_empty() { String::new() } else { tr!(" (for single steps also: {probes})", probes = v.join(", ")) }
}

/// Today's date (UTC), as dd.mm.yyyy.
fn today() -> String {
    let secs = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{d:02}.{m:02}.{y}")
}
