//! The CMM workspace (next to Design and Render): its own ribbon tab with the setup,
//! characteristic and output commands, the CMM Plan in the browser, and the active setup's
//! coordinate system drawn on the part. The plans themselves are design data (`cmm.*`
//! commands); this module only switches the workspace and shows them.

use std::sync::Arc;

use serde_json::Value;
use solidcraft_engine::inspection as insp;

use crate::SolveApp;

/// The tab the workspace shows.
pub const TAB: &str = "CMM";

/// The plans as the tree and the overlay show them, read again when the design changes.
#[derive(Default)]
pub struct State {
    key: Option<(u64, usize)>,
    summary: Value,
    /// The characteristic picked in the tree: its probing points and the probe are shown.
    pub focus: Option<u64>,
    /// The alignment element picked in the tree (setup, its index in the alignment's
    /// elements): its probing points and the probe are shown, with or without a measurement
    /// of its own.
    pub focus_alignment: Option<(u64, usize)>,
    /// The markers of the open dialog's characteristic, by what they were made from.
    preview: Option<(String, Value)>,
    /// The measured characteristic picked in the tree: its points are drawn larger and labelled.
    pub focus_result: Option<String>,
}

pub fn active(app: &SolveApp) -> bool {
    app.ui.tab == TAB
}

/// The workspace shown: `design`, `render` or `cmm`.
pub fn current(app: &SolveApp) -> &'static str {
    if crate::render_workspace::active(app) {
        "render"
    } else if active(app) {
        "cmm"
    } else {
        "design"
    }
}

/// Switch to a workspace by name (`design`, `render`, `cmm`).
pub fn switch(app: &mut SolveApp, workspace: &str) -> Result<(), String> {
    match workspace {
        "render" => {
            if active(app) {
                app.ui.tab = "MODEL".into();
            }
            crate::render_workspace::enter(app, true)
        }
        "cmm" => {
            if active(app) {
                return Ok(());
            }
            if app.session.active_sketch.is_some() || app.tool.is_some() || app.dialog.is_some() || app.file_load.busy() {
                return Err("finish the current modelling operation before entering CMM".into());
            }
            if crate::render_workspace::active(app) {
                crate::render_workspace::enter(app, false)?;
            }
            app.ui.tab = TAB.into();
            app.cancel_view_animation();
            Ok(())
        }
        "design" => {
            if crate::render_workspace::active(app) {
                return crate::render_workspace::enter(app, false);
            }
            if active(app) {
                app.dialog = None;
                app.ui.tab = "MODEL".into();
            }
            Ok(())
        }
        other => Err(format!("unknown workspace {other:?} (design, render or cmm)")),
    }
}

/// The plans described (`solidcraft_engine::inspection::summary`), cached until the design
/// changes.
pub fn summary(app: &mut SolveApp) -> Value {
    let key = (app.session.revision, Arc::as_ptr(&app.session.doc) as usize);
    if app.cmm.key != Some(key) {
        let model = app.session.world_state();
        app.cmm.summary = insp::summary(&model, &app.session.doc.cmm, app.session.path.as_deref());
        app.cmm.key = Some(key);
    }
    app.cmm.summary.clone()
}

/// Writes a setup's inspection plan (the active one by default), replacing an earlier plan
/// folder of the same name, and says where it went and what it left out.
/// Create Inspection Plan: the setup's NC program dialog (its first program, or a new one when
/// it has none yet); OK there saves the program and writes the plan.
pub fn export(app: &mut SolveApp, setup: Option<u64>) {
    let Some(st) = setup.and_then(|s| app.session.doc.cmm.setup(s)).or_else(|| app.session.doc.cmm.active_setup()) else {
        app.set_status(crate::i18n::tr_label("create a setup first (CMM → New Setup)").to_string(), true);
        return;
    };
    let (id, nc) = (st.id, st.programs.first().map(|p| p.id));
    crate::dialogs_cmm::edit_nc(app, id, nc);
}

/// Shows what writing a plan gave (`cmm.export`'s answer) in the status bar.
pub fn show_export(app: &mut SolveApp, v: &Value) {
    if !v.is_object() {
        return;
    }
    let mut msg = format!("{}: {}", crate::i18n::tr_label("Inspection plan written"), v["folder"].as_str().unwrap_or_default());
    let skipped: Vec<&str> = v["skipped"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !skipped.is_empty() {
        msg.push_str(&format!(" — {}: {}", crate::i18n::tr_label("not exported yet"), skipped.join(", ")));
    }
    if let Some(sheet) = v["sheet"].as_str() {
        let name = std::path::Path::new(sheet).file_name().and_then(|n| n.to_str()).unwrap_or(sheet);
        msg.push_str(&format!(" · {}: {name}", crate::i18n::tr_label("Setup sheet")));
    }
    if let Some(step) = v["step"].as_str() {
        let name = std::path::Path::new(step).file_name().and_then(|n| n.to_str()).unwrap_or(step);
        msg.push_str(&format!(" · {}: {name}", crate::i18n::tr_label("CAD model")));
    }
    let warnings: Vec<&str> = v["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !warnings.is_empty() {
        msg.push_str(&format!(" — {}", warnings.join("; ")));
    }
    app.set_status(msg, false);
}

/// Load Measurement Results: the NC program's postprocessor reads the results the measuring
/// software saved in the plan's folder; they are shown on the part and in the tree.
pub fn load_results(app: &mut SolveApp, setup: Option<u64>, nc: Option<u64>) {
    let mut p = serde_json::json!({});
    if let Some(s) = setup {
        p["setup"] = s.into();
    }
    if let Some(n) = nc {
        p["nc"] = n.into();
    }
    match app.run("cmm.results", p) {
        Ok(v) => {
            app.cmm.focus_result = None;
            let msg = format!(
                "{}: {} · {} {} · {} {}",
                crate::i18n::tr_label("Measurement results"),
                v["plan"].as_str().unwrap_or_default(),
                v["in_tolerance"].as_u64().unwrap_or(0),
                crate::i18n::tr_label("in tolerance"),
                v["out_of_tolerance"].as_u64().unwrap_or(0),
                crate::i18n::tr_label("out of tolerance"),
            );
            app.set_status(msg, false);
        }
        Err(e) => app.set_status(e, true),
    }
}

/// A grade's colour.
pub fn grade_color(g: solidcraft_engine::inspection_results::Grade) -> egui::Color32 {
    use solidcraft_engine::inspection_results::Grade;
    let t = crate::theme::Tokens::get();
    match g {
        Grade::Good => t.agent_ok,
        Grade::Near => t.warning,
        Grade::Out => t.error,
        Grade::Unknown => t.accent,
    }
}

/// Draws the loaded measurement results: every probed point coloured by its deviation, with
/// the deviation beside it (always for points outside their tolerance and the picked
/// characteristic's), and a legend.
fn draw_results(app: &SolveApp, painter: &egui::Painter, proj: &crate::viewport::Proj) {
    use solidcraft_engine::inspection_results::Grade;
    let Some(r) = app.session.cmm_results.as_ref() else { return };
    let t = crate::theme::Tokens::get();
    let focus = app.cmm.focus_result.as_deref();
    let few = r.points.len() <= 30;
    let font = egui::FontId::proportional(t.font_small);
    // labels never cover each other: each takes the first free corner around its point, else
    // stays away (the picked characteristic's always shows)
    let mut taken: Vec<egui::Rect> = Vec::new();
    let mut labels: Vec<(egui::Pos2, f32, String, bool)> = Vec::new();
    for p in r.points.iter().take(20_000) {
        let Some(at) = proj.to_screen(v3(p.at)) else { continue };
        let grade = p.grade();
        let color = grade_color(grade);
        let focused = focus.is_some_and(|f| p.characteristic.as_deref() == Some(f) || p.element == f);
        // a short stem out of the surface, against the probing direction
        let out = [p.at[0] - p.dir[0] * 3.0, p.at[1] - p.dir[1] * 3.0, p.at[2] - p.dir[2] * 3.0];
        if let Some(q) = proj.to_screen(v3(out)) {
            painter.line_segment([at, q], egui::Stroke::new(1.5, color));
        }
        let radius = if focused { 7.0 } else { 4.5 };
        painter.circle(at, radius, color, egui::Stroke::new(if focused { 2.0 } else { 1.0 }, if focused { t.text } else { t.panel }));
        taken.push(egui::Rect::from_center_size(at, egui::vec2(radius * 2.0, radius * 2.0)));
        if let Some(d) = p.deviation
            && (few || focused || grade == Grade::Out)
        {
            labels.push((at, radius, format!("{d:+.3}"), focused));
        }
    }
    // the picked characteristic's labels first, then the rest
    labels.sort_by_key(|l| !l.3);
    for (at, radius, text, focused) in labels {
        let galley = painter.layout_no_wrap(text, font.clone(), t.text);
        let size = galley.size() + egui::vec2(4.0, 4.0);
        let gap = radius + 2.0;
        let corners = [
            (egui::Align2::LEFT_BOTTOM, egui::vec2(gap, -gap)),
            (egui::Align2::LEFT_TOP, egui::vec2(gap, gap)),
            (egui::Align2::RIGHT_BOTTOM, egui::vec2(-gap, -gap)),
            (egui::Align2::RIGHT_TOP, egui::vec2(-gap, gap)),
        ];
        let free = corners.iter().map(|(align, off)| align.anchor_size(at + *off, size)).find(|rect| !taken.iter().any(|o| o.intersects(*rect)));
        let Some(rect) = free.or_else(|| focused.then(|| corners[0].0.anchor_size(at + corners[0].1, size))) else { continue };
        taken.push(rect);
        painter.rect_filled(rect, 3.0, t.panel.gamma_multiply(0.85));
        painter.galley(rect.min + egui::vec2(2.0, 2.0), galley, t.text);
    }
    // legend, bottom right
    let clip = painter.clip_rect();
    let count = |g: Grade| r.points.iter().filter(|p| p.grade() == g).count();
    let mut lines: Vec<(Option<Grade>, String)> = vec![(
        None,
        format!(
            "{} {}{}",
            crate::i18n::tr_label("Measurement results"),
            r.plan,
            r.measured.as_deref().map(|m| format!(" · {}", m.replace('T', " "))).unwrap_or_default()
        ),
    )];
    for (g, label) in [
        (Grade::Good, "within ¾ of the tolerance"),
        (Grade::Near, "within the tolerance"),
        (Grade::Out, "out of tolerance"),
        (Grade::Unknown, "no tolerance"),
    ] {
        lines.push((Some(g), format!("{}: {}", crate::i18n::tr_label(label), count(g))));
    }
    let galleys: Vec<_> =
        lines.into_iter().map(|(g, text)| (g, painter.layout_no_wrap(text, font.clone(), if g.is_some() { t.text_dim } else { t.text }))).collect();
    let row = t.font_small + 6.0;
    let width = galleys.iter().map(|(g, x)| x.size().x + if g.is_some() { 28.0 } else { 10.0 }).fold(0.0, f32::max) + 12.0;
    let height = row * galleys.len() as f32 + 10.0;
    let rect = egui::Rect::from_min_size(egui::pos2(clip.right() - width - 16.0, clip.bottom() - height - 64.0), egui::vec2(width, height));
    painter.rect_filled(rect, 6.0, t.panel.gamma_multiply(0.92));
    painter.rect_stroke(rect, 6.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let mut y = rect.top() + 5.0 + row / 2.0;
    for (g, galley) in galleys {
        let x = match g {
            Some(g) => {
                painter.circle(egui::pos2(rect.left() + 16.0, y), 4.5, grade_color(g), egui::Stroke::new(1.0, t.panel));
                rect.left() + 28.0
            }
            None => rect.left() + 10.0,
        };
        let h = galley.size().y;
        painter.galley(egui::pos2(x, y - h / 2.0), galley, t.text);
        y += row;
    }
}

fn arr3(v: &Value) -> Option<[f64; 3]> {
    Some([v.get(0)?.as_f64()?, v.get(1)?.as_f64()?, v.get(2)?.as_f64()?])
}

/// The coordinate system to draw: the one the open Alignment dialog picks make, else the
/// active setup's (in the CMM workspace).
fn shown_frame(app: &mut SolveApp) -> Option<(insp::Frame, String, bool)> {
    if let Some(d) = &app.dialog
        && matches!(d.kind, crate::dialogs::Kind::Cmm(crate::dialogs_cmm::Km::Alignment { .. }))
    {
        let a = crate::dialogs_cmm::alignment_of(
            app,
            &d.inputs,
            crate::dialogs_cmm::clamp_of(match &d.kind {
                crate::dialogs::Kind::Cmm(km) => km,
                _ => return None,
            }),
        );
        if a.is_complete() {
            return insp::frame(&app.session.world_state(), &a).ok().map(|f| (f, String::new(), true));
        }
        return None;
    }
    if !active(app) {
        return None;
    }
    let s = summary(app);
    let setup = s["setups"].as_array()?.iter().find(|x| x["active"] == true)?.clone();
    let f = &setup["alignment"]["frame"];
    let frame = insp::Frame { origin: arr3(&f["origin"])?, x: arr3(&f["x"])?, y: arr3(&f["y"])?, z: arr3(&f["z"])? };
    Some((frame, setup["name"].as_str().unwrap_or_default().to_string(), false))
}

/// The probing markers to show (the open dialog's characteristic, else the one picked in the
/// tree), and the probe's dimensions.
fn shown_markers(app: &mut SolveApp) -> Option<(Value, Value)> {
    let probe = |app: &mut SolveApp| {
        let s = summary(app);
        s["setups"]
            .as_array()
            .and_then(|v| v.iter().find(|x| x["active"] == true))
            .map(|st| st["probe"]["probe_geometry"].clone())
            .unwrap_or(Value::Null)
    };
    if let Some(d) = &app.dialog {
        let crate::dialogs::Kind::Cmm(k) = &d.kind else { return None };
        let c = crate::dialogs_cmm::characteristic_of(k, &d.inputs)?;
        let key = format!("{}:{}", app.session.revision, serde_json::to_string(&c).unwrap_or_default());
        if app.cmm.preview.as_ref().is_none_or(|(k, _)| *k != key) {
            let model = app.session.world_state();
            let mut m = app.session.doc.cmm.active_setup().map_or(Value::Null, |st| insp::characteristic_markers(&model, st, &c));
            // too few points picked yet: show those there are
            if m.as_array().is_none_or(Vec::is_empty) {
                let picks = match &c.kind {
                    solidcraft_engine::doc::cmm::CharacteristicKind::Plane { picks, .. }
                    | solidcraft_engine::doc::cmm::CharacteristicKind::Cylinder { picks, .. } => picks.clone(),
                    _ => Vec::new(),
                };
                m = Value::Array(picks.iter().map(|p| serde_json::json!({"type": "point", "at": p, "normal": [0.0, 0.0, 0.0]})).collect());
            }
            app.cmm.preview = Some((key, m));
        }
        let m = app.cmm.preview.as_ref().map(|(_, m)| m.clone()).unwrap_or(Value::Null);
        // the probe the dialog chose, else the setup's
        let own = app.dialog.as_ref().and_then(|d| d.extra.get("probe_file")).and_then(Value::as_str).and_then(insp::probe_row);
        let g = match own {
            Some(row) => serde_json::to_value(row).unwrap_or(Value::Null),
            None => probe(app),
        };
        return Some((m, g));
    }
    if !active(app) {
        return None;
    }
    if let Some((sid, i)) = app.cmm.focus_alignment {
        let s = summary(app);
        let st = s["setups"].as_array()?.iter().find(|x| x["id"] == sid)?.clone();
        let e = st["alignment"]["elements"].get(i)?.clone();
        let own = e["probe_file"].as_str().and_then(insp::probe_row).and_then(|r| serde_json::to_value(r).ok());
        return Some((e["markers"].clone(), own.unwrap_or_else(|| st["probe"]["probe_geometry"].clone())));
    }
    let focus = app.cmm.focus?;
    let s = summary(app);
    let st = s["setups"].as_array()?.iter().find(|x| x["characteristics"].as_array().is_some_and(|c| c.iter().any(|c| c["id"] == focus)))?.clone();
    let c = st["characteristics"].as_array()?.iter().find(|c| c["id"] == focus)?.clone();
    let own = c["probe_file"].as_str().and_then(insp::probe_row).and_then(|r| serde_json::to_value(r).ok());
    Some((c["markers"].clone(), own.unwrap_or_else(|| st["probe"]["probe_geometry"].clone())))
}

fn v3(a: [f64; 3]) -> solidcraft_engine::geom::Vec3 {
    solidcraft_engine::geom::Vec3::new(a[0], a[1], a[2])
}

/// Screen radius of a world radius `r` around `p` (seen across a vertical axis).
fn screen_radius(proj: &crate::viewport::Proj, p: [f64; 3], r: f64) -> Option<f32> {
    let c = proj.to_screen(v3(p))?;
    let a = proj.to_screen(v3([p[0] + r, p[1], p[2]])).map_or(0.0, |q| (q - c).length());
    let b = proj.to_screen(v3([p[0], p[1] + r, p[2]])).map_or(0.0, |q| (q - c).length());
    Some(a.max(b).max(1.0))
}

/// Draws the probe standing on a probing point: the ball touching the surface, the shafts and
/// the base body up along +Z (dimensions from the probe table, else typical ones).
fn draw_probe(painter: &egui::Painter, proj: &crate::viewport::Proj, at: [f64; 3], normal: [f64; 3], g: &Value) {
    let t = crate::theme::Tokens::get();
    let num = |k: &str, d: f64| g[k].as_f64().filter(|x| x.is_finite() && *x > 0.0).unwrap_or(d);
    let ball = num("ball", 2.0);
    let r = ball / 2.0;
    let ml = num("measuring_length", 31.0);
    let len = num("length", 40.0).max(ml);
    let shaft = num("shaft", 1.5).min(ball);
    let body = num("body", 5.0);
    let c = [at[0] + normal[0] * r, at[1] + normal[1] * r, at[2] + normal[2] * r];
    let up = |h: f64| [c[0], c[1], c[2] + h];
    let mut parts: Vec<(f64, f64, f64)> = Vec::new();
    match (g["measuring_length2"].as_f64(), g["shaft2"].as_f64()) {
        (Some(m2), Some(d2)) if m2 > 0.0 && m2 < ml && d2 > 0.0 => {
            parts.push((0.0, m2, d2));
            parts.push((m2, ml, shaft));
        }
        _ => parts.push((0.0, ml, shaft)),
    }
    parts.push((ml, len, body));
    let fill = t.text.gamma_multiply(0.35);
    let edge = egui::Stroke::new(1.0, t.text.gamma_multiply(0.8));
    for (h0, h1, d) in parts {
        let (Some(p0), Some(p1)) = (proj.to_screen(v3(up(h0))), proj.to_screen(v3(up(h1)))) else { continue };
        let (Some(w0), Some(w1)) = (screen_radius(proj, up(h0), d / 2.0), screen_radius(proj, up(h1), d / 2.0)) else { continue };
        let dir = (p1 - p0).normalized();
        let side = if dir.is_finite() { egui::vec2(-dir.y, dir.x) } else { egui::vec2(1.0, 0.0) };
        let quad = vec![p0 + side * w0, p1 + side * w1, p1 - side * w1, p0 - side * w0];
        painter.add(egui::Shape::convex_polygon(quad, fill, edge));
    }
    if let (Some(pc), Some(w)) = (proj.to_screen(v3(c)), screen_radius(proj, c, r)) {
        painter.circle(pc, w, t.accent.gamma_multiply(0.85), egui::Stroke::new(1.0, t.text));
    }
}

/// Draws the probing points and paths of the shown characteristic, numbered, and the probe at
/// the first point.
fn draw_markers(app: &mut SolveApp, painter: &egui::Painter, proj: &crate::viewport::Proj) {
    let Some((markers, probe)) = shown_markers(app) else { return };
    let Some(list) = markers.as_array() else { return };
    let t = crate::theme::Tokens::get();
    let stroke = egui::Stroke::new(2.0, t.accent);
    let mut first: Option<([f64; 3], [f64; 3])> = None;
    let mut n = 0;
    for m in list.iter().take(2000) {
        let normal = arr3(&m["normal"]).unwrap_or([0.0, 0.0, 1.0]);
        match m["type"].as_str() {
            Some("point") => {
                let Some(at) = arr3(&m["at"]) else { continue };
                n += 1;
                first.get_or_insert((at, normal));
                let Some(p) = proj.to_screen(v3(at)) else { continue };
                let tip = proj.to_screen(v3([at[0] + normal[0] * 4.0, at[1] + normal[1] * 4.0, at[2] + normal[2] * 4.0]));
                if let Some(q) = tip {
                    painter.line_segment([p, q], egui::Stroke::new(1.5, t.accent));
                }
                painter.circle(p, 4.0, t.accent, egui::Stroke::new(1.0, t.panel));
                if list.len() <= 64 {
                    painter.text(
                        p + egui::vec2(6.0, -6.0),
                        egui::Align2::LEFT_BOTTOM,
                        n.to_string(),
                        egui::FontId::proportional(t.font_small),
                        t.text,
                    );
                }
            }
            Some("circle") => {
                let (Some(c), Some(r)) = (arr3(&m["center"]), m["radius"].as_f64()) else { continue };
                // two axes across the normal
                let pick = if normal[2].abs() > 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
                let d = pick[0] * normal[0] + pick[1] * normal[1] + pick[2] * normal[2];
                let mut u = [pick[0] - d * normal[0], pick[1] - d * normal[1], pick[2] - d * normal[2]];
                let l = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt().max(1e-12);
                u = u.map(|x| x / l);
                let w = [normal[1] * u[2] - normal[2] * u[1], normal[2] * u[0] - normal[0] * u[2], normal[0] * u[1] - normal[1] * u[0]];
                let at = |a: f64| {
                    [
                        c[0] + r * (a.cos() * u[0] + a.sin() * w[0]),
                        c[1] + r * (a.cos() * u[1] + a.sin() * w[1]),
                        c[2] + r * (a.cos() * u[2] + a.sin() * w[2]),
                    ]
                };
                let pts: Vec<egui::Pos2> = (0..=48).filter_map(|k| proj.to_screen(v3(at(f64::from(k) * std::f64::consts::TAU / 48.0)))).collect();
                if pts.len() > 1 {
                    painter.add(egui::Shape::line(pts, stroke));
                }
                // a path's start (at +u): on a plane the ball rests on it, in a bore it touches the wall
                let start = at(0.0);
                let n = arr3(&m["start_normal"]).unwrap_or(normal);
                first.get_or_insert((start, n));
            }
            _ => {}
        }
    }
    if let Some((at, normal)) = first {
        draw_probe(painter, proj, at, normal, &probe);
    }
}

/// Draws the coordinate system (called by the viewport after the model is drawn).
pub fn overlay(app: &mut SolveApp, painter: &egui::Painter, proj: &crate::viewport::Proj) {
    draw_results(app, painter, proj);
    draw_markers(app, painter, proj);
    let Some((f, name, preview)) = shown_frame(app) else { return };
    let t = crate::theme::Tokens::get();
    let o = solidcraft_engine::geom::Vec3::new(f.origin[0], f.origin[1], f.origin[2]);
    let len = app.cam.half_height() * 0.22;
    let Some(po) = proj.to_screen(o) else { return };
    let width = if preview { 2.0 } else { 3.0 };
    for (axis, color, letter) in [(f.x, t.axis_x, "X"), (f.y, t.axis_y, "Y"), (f.z, t.axis_z, "Z")] {
        let d = solidcraft_engine::geom::Vec3::new(axis[0], axis[1], axis[2]);
        let Some(pe) = proj.to_screen(o + d * len) else { continue };
        painter.line_segment([po, pe], egui::Stroke::new(width, color));
        // arrow head
        let dir = (pe - po).normalized();
        if dir.is_finite() {
            let side = egui::vec2(-dir.y, dir.x) * 5.0;
            let back = pe - dir * 11.0;
            painter.add(egui::Shape::convex_polygon(vec![pe, back + side, back - side], color, egui::Stroke::NONE));
        }
        painter.text(pe + dir * 9.0, egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(t.font_label), color);
    }
    painter.circle(po, 4.5, t.panel, egui::Stroke::new(2.0, t.accent));
    if !name.is_empty() {
        painter.text(po + egui::vec2(10.0, 10.0), egui::Align2::LEFT_TOP, name, egui::FontId::proportional(t.font_small), t.text);
    }
}

#[cfg(test)]
#[path = "cmm_workspace_tests.rs"]
mod tests;
