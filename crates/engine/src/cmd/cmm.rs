//! Coordinate measuring plans (`cmm.*`), saved with the design and written as a measuring
//! machine's inspection plan by a postprocessor (see [`crate::inspection`]).
//!
//! A plan is a list of setups. Each setup has its probe system (`cmm.setup`), its alignment
//! (`cmm.alignment`) and its measurements and characteristics (`cmm.circle`, `cmm.cylinder`,
//! `cmm.plane`, `cmm.distance`, `cmm.angle`, `cmm.point`). Faces are `{body, point}`; a characteristic
//! command without them takes the selected faces in order. Characteristics go into the active
//! setup unless `setup` names another; `characteristic` edits one in place.

use crate::tr;
use serde_json::{Value, json};

use super::CommandSpec;
use crate::inspection::{
    self as insp, AlignBores, Alignment, Axis, Characteristic, CharacteristicKind, CylinderProbing, DirectionRef, FaceKind, FaceRef,
    MAX_CHARACTERISTICS, MAX_PICKS, MAX_SETUPS, OriginRef, PlaneProbing, SafetyPlane, Setup,
};
use crate::params::bad;
use crate::{EngineError, Result, Sel, Session};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cmm.setup", "New Setup", setup).at("CMM", "SETUP").icon("new").doc_only().params(
        "setup?: id (edits that setup; without it a new one, made active); name?: text; probe_file?: a probe table row's name \
         (or a probe file's path), or null (the post's default); safety_plane?: +z|-z|+x|-x|+y|-y|auto (where the probe retracts \
         between elements; auto: the side each face looks to; default +z); plane_probing?: points|circles (how planes are probed \
         unless a plane measurement says otherwise; default points) → {setup}. Postprocessor and plan output: cmm.nc",
    ),
    CommandSpec::new("cmm.alignment", "Alignment", alignment).at("CMM", "SETUP").icon("origin").enabled(has_setup).params(
        "setup?: id (default: the active one); primary?: face (a plane: spatial axis and Z, out of the face; the part may lie turned \
         in the model: the measuring axes follow the primary and direction faces); clamp?: bool (the primary face is the face the \
         part is clamped on: Z points into the part and the clamping plate beside the part is probed instead of the face); \
         clamp_points?: [[x, y], …] or null (points on the plate in the alignment's X/Y, at least 3, none under the part; null or \
         empty: four points outside the part's corners); clamp_margin?: mm (how far outside, default 10); direction?: [face] (a side plane) or \
         [face, face] (two bores: X runs from the first to the second); origin_x?, origin_y?: [face] (a side plane or a bore) or [face, \
         face] (the middle of two opposite sides); a bore for X alone is also Y; bores?: cylinder|circle (how the alignment's \
         bores are measured: a cylinder whose axis meets the primary plane, or a circle below the opening whose centre is the \
         origin; a circle characteristic on the bore gives its points and height; default cylinder); clear?: bool. Faces are \
         {body, point}",
    ),
    CommandSpec::new("cmm.circle", "Circle", circle).at("CMM", "CHARACTERISTICS").icon("circle3").enabled(has_setup).params(
        "face?: bore {body, point} (default: the selected face); points?: 3–100 (4); height?: mm below the bore's opening plane (2); \
         position?: bool (X, Y from the origin too); tol?: ± mm (0.05); safety_plane?: +z|…|auto|null (null: the setup's); probe_file?: \
         path of a probe configuration, a probe table name, or null (the setup's; every measurement and characteristic takes it); name?; setup?; \
         characteristic?: id (edit)",
    ),
    CommandSpec::new("cmm.cylinder", "Cylinder Measurement", cylinder).at("CMM", "CHARACTERISTICS").icon("cylinder").enabled(has_setup).params(
        "face?: bore {body, point} (default: the selected face); strategy?: scan|circle_points|points (scan); paths?: 1–20 circular \
         paths along the depth (2); points?: 3–100 per path for circle_points (7); height?: mm the first and last path keep from the \
         bore's ends (2); picks?: [[x, y, z]] points on the wall for strategy points (at least 4); tol?: ± mm (0.05); safety_plane?; \
         name?; setup?; characteristic?: id (edit). Its strategy also holds where the alignment or a distance uses the bore",
    ),
    CommandSpec::new("cmm.plane", "Plane Measurement", plane).at("CMM", "CHARACTERISTICS").icon("plane").enabled(has_setup).params(
        "face?: plane {body, point} (default: the selected face); strategy?: points|circles (points); points?: 3–100 spread over the \
         face when none are picked (4); picks?: [[x, y, z]] points picked on the face (at least 4); safety_plane?; name?; setup?; \
         characteristic?: id (edit). Says how the plane is probed wherever it is used (alignment, distances); evaluates nothing itself",
    ),
    CommandSpec::new("cmm.distance", "Distance", distance)
        .at("CMM", "RELATIONS")
        .icon("measure")
        .enabled(has_setup)
        .params("from?, to?: plane or bore {body, point} (default: the first two selected faces); axis?: x|y|z (default: across the planes, else the bores' larger offset); tol?: ± mm (0.05); name?; setup?; characteristic?: id (edit)"),
    CommandSpec::new("cmm.angle", "Angle", angle)
        .at("CMM", "RELATIONS")
        .icon("plane_angle")
        .enabled(has_setup)
        .params("first?, second?: planes {body, point} (default: the first two selected faces); tol?: ± degrees (0.1); name?; setup?; characteristic?: id (edit)"),
    CommandSpec::new("cmm.point", "Points", point).at("CMM", "CHARACTERISTICS").icon("point").enabled(has_setup).params(
        "points?: [{body, point}] one or many points on the part (e.g. along a 3D contour; default: the selected face's pick); face?: \
         one point (older form); check?: normal (each point's deviation along the surface normal: the form of a contour; default) | \
         coordinates (X, Y, Z of each point from the origin); tol?: ± mm for every point (0.05); name?; setup?; characteristic?: id (edit)",
    ),
    CommandSpec::new("cmm.export", "Create Inspection Plan", export).at("CMM", "OUTPUT").icon("export").noundo().enabled(has_setup).params(
        "setup?: id (default: the active one); nc?: id (its NC program; default: the first, or the setup's own post settings \
         without any); dir?: folder (default: the NC program's output folder); replace?: bool (replace an existing plan folder) → folder, plan, program (the neutral program written beside it), post, skipped (what the post does not write yet), \
         warnings. Runs the setup's postprocessor on the neutral program (docs/cmm-postprocessors.md)",
    ),
    CommandSpec::new("cmm.nc", "NC Program", nc_program).at("CMM", "OUTPUT").icon("export").enabled(has_setup).params(
        "nc?: id (edits that NC program; without it a new one in the setup); setup?: id (default: the active one); name?: text; \
         post?: a postprocessor's name in <data>/inspection/post or its path, or null (the first there); plan_name?: text (empty: \
         design and setup name, plus the program's after the first); output_dir?: folder or null (<data>/inspection/output); \
         options?: {key: text|bool} the post's settings (cmm.post.options lists them; replaces the program's); characteristics?: \
         [id] the characteristics it outputs, or null (all); sheet?: bool (write the setup sheet PDF beside the plan; default \
         true); step?: bool (write the measured bodies as <plan>.step in the plan's base system beside the plan, for the CAD \
         import of the measuring software; default true); export?: bool (write its plan right after, replacing an existing one) \
         → {setup, nc, export}",
    ),
    CommandSpec::new("cmm.nc.delete", "Delete NC Program", delete_nc).icon("delete").doc_only().params("nc: id"),
    CommandSpec::new("cmm.sheet", "Setup Sheet", sheet).icon("export").noundo().enabled(has_setup).params(
        "setup?: id; nc?: id (its NC program; default: the first); path?: PDF file (default: <plan>_setup-sheet.pdf, German <plan>_Einrichtblatt.pdf, in the NC program's \
         output folder; into the plan's folder there once it exists) → {path, bytes}: the setup sheet for the technician: the part, plan and probe, an overview of the alignment, one \
         picture per alignment step in the setup's order with its probing points, and the characteristics. cmm.export writes it too",
    ),
    CommandSpec::new("cmm.results", "Load Measurement Results", results).at("CMM", "OUTPUT").icon("import").noundo().enabled(has_setup).params(
        "setup?: id; nc?: id (its NC program; default: the first); path?: the plan's folder the measuring software saved its \
         results in (default: <output folder>/<plan>, where cmm.export wrote the plan); clear?: bool (hide the results) → {setup, nc, \
         plan, folder, measured, points, points_good, points_near, points_out, characteristics: [{name, actual, nominal, upper, \
         lower, unit, state: in|out|unknown}], in_tolerance, out_of_tolerance}: the NC program's postprocessor reads the results \
         (`--results <folder>`); the view shows every probed point coloured by its deviation from the nominal surface (green \
         within ¾ of its tolerance, yellow within it, red outside, blue without a tolerance). View state, not saved",
    ),
    CommandSpec::new("cmm.post.options", "Postprocessor Settings", post_options).icon("settings").noundo().params(
        "post?: a postprocessor's name or path (default: the first) → {post, options: [{key, label, kind: bool|text, default}]}: \
         what the post describes with `--describe`",
    ),
    CommandSpec::new("cmm.program", "CMM Program", program).icon("export").noundo().enabled(has_setup).params(
        "setup?: id (default: the active one); nc?: id (its NC program: its characteristics, plan name and post settings); \
         path?: file to write it to (JSON) → the setup's neutral CMM program \
         (solidcraft-cmm-program/1, docs/cmm-postprocessors.md), what a postprocessor translates",
    ),
    CommandSpec::new("cmm.probe.table", "Probe Table", probe_table).at("CMM", "SETUP").icon("configuration_table").noundo().params(
        "rows?: [{file, ball, length, measuring_length, shaft, measuring_length2, shaft2, body, order_no, note}] (mm; file: the probe \
         file's name without its extension) saves the table → {path, rows}: the probe table in <data>/inspection/probes/probe-table.json, with \
         an empty row for every probe file without one",
    ),
    CommandSpec::new("cmm.state", "CMM Plans", state)
        .icon("inspect")
        .noundo()
        .params("→ setups [{id, name, active, probe, alignment {set, complete, problem, frame}, characteristics, ready, problem}], active, posts, probe_files, data_folder"),
    CommandSpec::new("cmm.setup.activate", "Activate Setup", activate).icon("activate").doc_only().params("setup: id"),
    CommandSpec::new("cmm.setup.delete", "Delete Setup", delete_setup).icon("delete").doc_only().params("setup: id"),
    CommandSpec::new("cmm.characteristic.delete", "Delete Characteristic", delete_characteristic)
        .icon("delete")
        .doc_only()
        .params("characteristic: id; setup?: id"),
    CommandSpec::new("cmm.rename", "Rename CMM Item", rename).icon("rename").doc_only().params("setup?: id; characteristic?: id; name: text (empty: automatic, for a characteristic)"),
];

fn other(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

fn has_setup(s: &Session) -> std::result::Result<(), String> {
    if s.doc.cmm.setups.is_empty() { Err("create a setup first (CMM → New Setup)".into()) } else { Ok(()) }
}

fn state(s: &mut Session, _p: &Value) -> Result<Value> {
    let model = s.world_state();
    Ok(insp::summary(&model, &s.doc.cmm, s.path.as_deref()))
}

fn id_param(p: &Value, key: &str, cmd: &str) -> Result<Option<u64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be an id"))),
    }
}

/// The setup a command works on: `setup`, else the active one.
fn setup_id(s: &Session, p: &Value, cmd: &str) -> Result<u64> {
    match id_param(p, "setup", cmd)? {
        Some(id) if s.doc.cmm.setup(id).is_some() => Ok(id),
        Some(id) => Err(bad(cmd, format!("there is no setup {id}"))),
        None => s.doc.cmm.active_setup().map(|x| x.id).ok_or_else(|| bad(cmd, "create a setup first (CMM → New Setup)")),
    }
}

fn parse_face(v: &Value, cmd: &str) -> Result<FaceRef> {
    let body = v.get("body").and_then(Value::as_str).ok_or_else(|| bad(cmd, "a face needs `body`"))?;
    let point: [f64; 3] =
        v.get("point").and_then(|x| serde_json::from_value::<[f64; 3]>(x.clone()).ok()).ok_or_else(|| bad(cmd, "a face needs `point`: [x, y, z]"))?;
    let f = FaceRef { body: body.to_string(), point };
    if f.is_valid() { Ok(f) } else { Err(bad(cmd, "a face needs a body name and a finite point")) }
}

fn selected_face(s: &Session, n: usize) -> Option<FaceRef> {
    s.selection
        .iter()
        .filter_map(|x| match x {
            Sel::Face { body, point, .. } => Some(FaceRef { body: body.clone(), point: [point.x, point.y, point.z] }),
            _ => None,
        })
        .nth(n)
}

/// The face `key`, else the `n`th selected face.
fn face(s: &Session, p: &Value, key: &str, n: usize, cmd: &str) -> Result<FaceRef> {
    match p.get(key) {
        None | Some(Value::Null) => selected_face(s, n).ok_or_else(|| bad(cmd, format!("`{key}`: select a face on the model"))),
        Some(v) => parse_face(v, cmd),
    }
}

/// An optional list of one or two faces.
fn face_list(p: &Value, key: &str, cmd: &str) -> Result<Option<Vec<FaceRef>>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) if (1..=2).contains(&a.len()) => Ok(Some(a.iter().map(|v| parse_face(v, cmd)).collect::<Result<_>>()?)),
        Some(Value::Array(a)) if a.is_empty() => Ok(Some(Vec::new())),
        Some(_) => Err(bad(cmd, format!("`{key}` must be a list of one or two faces"))),
    }
}

fn text_param(p: &Value, key: &str, cmd: &str) -> Result<Option<String>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(String::new())),
        Some(Value::String(t)) if t.chars().count() <= 200 => Ok(Some(t.trim().to_string())),
        Some(_) => Err(bad(cmd, format!("`{key}` must be a text of at most 200 characters"))),
    }
}

fn path_param(p: &Value, key: &str, cmd: &str) -> Result<Option<Option<String>>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(t)) if t.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(t)) if t.len() < 4096 => Ok(Some(Some(t.trim().to_string()))),
        Some(_) => Err(bad(cmd, format!("`{key}` must be a path or null"))),
    }
}

fn reply(s: &Session, setup: u64, extra: Value) -> Result<Value> {
    let model = s.world_state();
    let mut v = match s.doc.cmm.setup(setup) {
        Some(st) => insp::setup_info(&model, &s.doc.cmm, st, s.path.as_deref()),
        None => json!({}),
    };
    v["setup"] = json!(setup);
    if let (Value::Object(m), Value::Object(e)) = (&mut v, extra) {
        m.extend(e);
    }
    Ok(v)
}

fn setup(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.setup";
    let editing = id_param(p, "setup", cmd)?;
    if let Some(id) = editing
        && s.doc.cmm.setup(id).is_none()
    {
        return Err(bad(cmd, format!("there is no setup {id}")));
    }
    if editing.is_none() && s.doc.cmm.setups.len() >= MAX_SETUPS {
        return Err(bad(cmd, format!("at most {MAX_SETUPS} setups")));
    }
    // the postprocessor and the plan output belong to the NC programs now
    for k in ["post", "plan_name", "output_dir", "options", "sort_by_probe"] {
        if p.get(k).is_some() {
            return Err(bad(cmd, format!("`{k}` belongs to the setup's NC programs now (cmm.nc)")));
        }
    }
    let name = text_param(p, "name", cmd)?;
    let probe = path_param(p, "probe_file", cmd)?;
    if let Some(Some(f)) = &probe
        && !insp::probe_choice_ok(f)
    {
        return Err(other(tr!("Probe {probe} is neither a file nor in the probe table", probe = f)));
    }
    let safety = match p.get("safety_plane") {
        None | Some(Value::Null) => None,
        Some(v) => Some(safety_value(v, cmd)?),
    };
    let plane_probing = match p.get("plane_probing") {
        None | Some(Value::Null) => None,
        Some(v) => Some(plane_strategy(v, cmd)?),
    };
    let plans = &mut s.doc_mut().cmm;
    let id = match editing {
        Some(id) => id,
        None => {
            let id = plans.next_id();
            let name = plans.next_setup_name();
            plans.setups.push(Setup::new(id, name));
            plans.active = Some(id);
            id
        }
    };
    if let Some(st) = plans.setup_mut(id) {
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            st.name = n;
        }
        if let Some(t) = probe {
            st.probe.probe_file = t;
        }
        if let Some(x) = safety {
            st.safety_plane = x;
        }
        if let Some(x) = plane_probing {
            st.plane_probing = x;
        }
    }
    reply(s, id, json!({}))
}

fn activate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.setup.activate";
    let id = id_param(p, "setup", cmd)?.ok_or_else(|| bad(cmd, "`setup` is required"))?;
    if s.doc.cmm.setup(id).is_none() {
        return Err(bad(cmd, format!("there is no setup {id}")));
    }
    if s.doc.cmm.active != Some(id) {
        s.doc_mut().cmm.active = Some(id);
    }
    reply(s, id, json!({}))
}

fn delete_setup(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.setup.delete";
    let id = id_param(p, "setup", cmd)?.ok_or_else(|| bad(cmd, "`setup` is required"))?;
    if s.doc.cmm.setup(id).is_none() {
        return Err(bad(cmd, format!("there is no setup {id}")));
    }
    let plans = &mut s.doc_mut().cmm;
    plans.setups.retain(|x| x.id != id);
    if plans.active == Some(id) {
        plans.active = plans.setups.last().map(|x| x.id);
    }
    Ok(json!({"deleted": id}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.rename";
    let name = text_param(p, "name", cmd)?.ok_or_else(|| bad(cmd, "`name` is required"))?;
    match id_param(p, "characteristic", cmd)? {
        Some(c) => {
            let sid = find_characteristic(s, p, c, cmd)?;
            if let Some(ch) = s.doc_mut().cmm.setup_mut(sid).and_then(|st| st.characteristics.iter_mut().find(|x| x.id == c)) {
                ch.name = name;
            }
            reply(s, sid, json!({"characteristic": c}))
        }
        None => {
            let sid = setup_id(s, p, cmd)?;
            if name.is_empty() {
                return Err(bad(cmd, "a setup needs a name"));
            }
            if let Some(st) = s.doc_mut().cmm.setup_mut(sid) {
                st.name = name;
            }
            reply(s, sid, json!({}))
        }
    }
}

/// The setup holding characteristic `c` (`setup` if given).
fn find_characteristic(s: &Session, p: &Value, c: u64, cmd: &str) -> Result<u64> {
    let given = id_param(p, "setup", cmd)?;
    s.doc
        .cmm
        .setups
        .iter()
        .find(|st| given.is_none_or(|g| g == st.id) && st.characteristics.iter().any(|x| x.id == c))
        .map(|st| st.id)
        .ok_or_else(|| bad(cmd, format!("there is no characteristic {c}")))
}

fn delete_characteristic(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.characteristic.delete";
    let c = id_param(p, "characteristic", cmd)?.ok_or_else(|| bad(cmd, "`characteristic` is required"))?;
    let sid = find_characteristic(s, p, c, cmd)?;
    if let Some(st) = s.doc_mut().cmm.setup_mut(sid) {
        st.characteristics.retain(|x| x.id != c);
    }
    reply(s, sid, json!({"deleted": c}))
}

// ------------------------------------------------------------------------------------------
// Alignment
// ------------------------------------------------------------------------------------------

/// A face's kind in the part axes of alignment `a` (a turned part has upright faces there).
fn kind_of(s: &Session, a: &Alignment, f: &FaceRef, cmd: &str) -> Result<FaceKind> {
    let model = s.world_state();
    insp::in_alignment(&model, a, || insp::analyze(&model, f)).map(|i| i.kind).map_err(|e| bad(cmd, e))
}

fn origin_ref(s: &Session, al: &Alignment, faces: &[FaceRef], letter: &str, cmd: &str) -> Result<Option<OriginRef>> {
    let model = s.world_state();
    Ok(match faces {
        [] => None,
        [f] => Some(match kind_of(s, al, f, cmd)? {
            FaceKind::Bore { .. } => OriginRef::Bore { face: f.clone() },
            FaceKind::Plane { .. } => OriginRef::Side { face: f.clone() },
        }),
        // two picks in the same bore (both halves of its wall, or one twice) are that bore
        [a, b] if insp::in_alignment(&model, al, || insp::same_bore(&model, a, b)) => Some(OriginRef::Bore { face: a.clone() }),
        [a, b] => Some(OriginRef::Middle { first: a.clone(), second: b.clone() }),
        _ => return Err(bad(cmd, format!("origin {letter}: one or two faces"))),
    })
}

fn alignment(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.alignment";
    let id = setup_id(s, p, cmd)?;
    let mut a = s.doc.cmm.setup(id).map(|st| st.alignment.clone()).unwrap_or_default();
    if p.get("clear").and_then(Value::as_bool) == Some(true) {
        a = Alignment::default();
    }
    match p.get("primary") {
        None => {}
        Some(Value::Null) => a.primary = None,
        Some(v) => a.primary = Some(parse_face(v, cmd)?),
    }
    if let Some(d) = face_list(p, "direction", cmd)? {
        a.direction = match d.as_slice() {
            [] => None,
            [f] => Some(DirectionRef::Side { face: f.clone() }),
            [x, y] => Some(DirectionRef::Bores { first: x.clone(), second: y.clone() }),
            _ => None,
        };
    }
    // the clamping plate as the primary plane
    match p.get("clamp") {
        None => {}
        Some(Value::Null) | Some(Value::Bool(false)) => a.clamp = None,
        Some(Value::Bool(true)) => {
            a.clamp.get_or_insert_with(insp::ClampPlane::default);
        }
        Some(_) => return Err(bad(cmd, "`clamp` must be true or false")),
    }
    match p.get("clamp_margin") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let m = v
                .as_f64()
                .filter(|m| m.is_finite() && *m > 0.0 && *m <= 1000.0)
                .ok_or_else(|| bad(cmd, "`clamp_margin` must be mm from 0 to 1000"))?;
            a.clamp.get_or_insert_with(insp::ClampPlane::default).margin = m;
        }
    }
    match p.get("clamp_points") {
        None => {}
        Some(Value::Null) => {
            if let Some(c) = a.clamp.as_mut() {
                c.points.clear();
            }
        }
        Some(Value::Array(list)) if list.len() <= insp::MAX_CLAMP_POINTS => {
            let mut pts = Vec::new();
            for q in list {
                let xy = q
                    .as_array()
                    .and_then(|q| Some([q.first()?.as_f64()?, q.get(1)?.as_f64()?]))
                    .filter(|q| q.iter().all(|c| c.is_finite() && c.abs() < 1e5))
                    .ok_or_else(|| bad(cmd, "`clamp_points` must be [[x, y], …] in mm"))?;
                pts.push(xy);
            }
            a.clamp.get_or_insert_with(insp::ClampPlane::default).points = pts;
        }
        Some(_) => return Err(bad(cmd, format!("`clamp_points` must be at most {} points [x, y]", insp::MAX_CLAMP_POINTS))),
    }
    match p.get("bores") {
        None | Some(Value::Null) => {}
        Some(v) => {
            a.bores = match v.as_str().map(str::trim).map(str::to_ascii_lowercase).as_deref() {
                Some("cylinder" | "zylinder") => AlignBores::Cylinder,
                Some("circle" | "kreis") => AlignBores::Circle,
                _ => return Err(bad(cmd, "`bores` must be cylinder or circle")),
            }
        }
    }
    let ox = face_list(p, "origin_x", cmd)?;
    let oy = face_list(p, "origin_y", cmd)?;
    if let Some(f) = &ox {
        a.origin_x = origin_ref(s, &a, f, "X", cmd)?;
    }
    if let Some(f) = &oy {
        a.origin_y = origin_ref(s, &a, f, "Y", cmd)?;
    }
    // a bore for X alone is the origin in Y too
    if oy.as_ref().is_none_or(Vec::is_empty)
        && let Some(OriginRef::Bore { face }) = &a.origin_x
        && (ox.is_some() || a.origin_y.is_none())
    {
        a.origin_y = Some(OriginRef::Bore { face: face.clone() });
    }
    let model = s.world_state();
    if a.is_complete() {
        insp::frame(&model, &a).map_err(|e| bad(cmd, e))?;
    } else {
        for f in a.primary.iter() {
            insp::in_alignment(&model, &a, || insp::analyze(&model, f)).map_err(|e| bad(cmd, e))?;
        }
    }
    if let Some(st) = s.doc_mut().cmm.setup_mut(id) {
        st.alignment = a;
    }
    reply(s, id, json!({}))
}

// ------------------------------------------------------------------------------------------
// Characteristics
// ------------------------------------------------------------------------------------------

/// Adds (or, with `characteristic`, replaces) a characteristic of `kind` after checking it
/// against the model.
fn put(s: &mut Session, p: &Value, cmd: &str, kind: CharacteristicKind, default_tol: f64, max_tol: f64) -> Result<Value> {
    let tol = match p.get("tol") {
        None | Some(Value::Null) => default_tol,
        Some(v) => v
            .as_f64()
            .filter(|t| t.is_finite() && (0.0..=max_tol).contains(t))
            .ok_or_else(|| bad(cmd, format!("`tol` must be a number from 0 to {max_tol}")))?,
    };
    let name = text_param(p, "name", cmd)?;
    let safety_plane = match p.get("safety_plane") {
        None | Some(Value::Null) => None,
        Some(v) => Some(safety_value(v, cmd)?),
    };
    let editing = id_param(p, "characteristic", cmd)?;
    let sid = match editing {
        Some(c) => find_characteristic(s, p, c, cmd)?,
        None => setup_id(s, p, cmd)?,
    };
    let id = editing.unwrap_or_else(|| s.doc.cmm.next_id());
    let old_name = editing.and_then(|c| s.doc.cmm.setup(sid)?.characteristics.iter().find(|x| x.id == c).map(|x| x.name.clone()));
    let probe_file = match path_param(p, "probe_file", cmd)? {
        Some(Some(f)) if !insp::probe_choice_ok(&f) => return Err(other(tr!("Probe {probe} is neither a file nor in the probe table", probe = f))),
        Some(f) => f,
        None => None,
    };
    let c = Characteristic { id, name: name.or(old_name).unwrap_or_default(), kind, tol, safety_plane, probe_file };
    let model = s.world_state();
    let al = s.doc.cmm.setup(sid).map(|st| st.alignment.clone()).unwrap_or_default();
    let info = insp::in_alignment(&model, &al, || insp::characteristic_info(&model, None, &c));
    if let Some(e) = info["error"].as_str() {
        return Err(bad(cmd, e));
    }
    if let Some(st) = s.doc.cmm.setup(sid) {
        insp::characteristic_elements(&model, st, &c).map_err(|e| bad(cmd, e))?;
    }
    let st = s.doc_mut().cmm.setup_mut(sid).ok_or_else(|| bad(cmd, "the setup is gone"))?;
    match st.characteristics.iter().position(|x| x.id == id) {
        Some(i) => {
            if let Some(slot) = st.characteristics.get_mut(i) {
                *slot = c;
            }
        }
        None if st.characteristics.len() >= MAX_CHARACTERISTICS => {
            return Err(bad(cmd, format!("at most {MAX_CHARACTERISTICS} characteristics per setup")));
        }
        None => st.characteristics.push(c),
    }
    reply(s, sid, json!({"characteristic": id}))
}

fn safety_value(v: &Value, cmd: &str) -> Result<SafetyPlane> {
    v.as_str().and_then(SafetyPlane::parse).ok_or_else(|| bad(cmd, "`safety_plane` must be +z, -z, +x, -x, +y, -y or auto"))
}

fn plane_strategy(v: &Value, cmd: &str) -> Result<PlaneProbing> {
    match v.as_str() {
        Some("points") => Ok(PlaneProbing::Points),
        Some("circles") => Ok(PlaneProbing::Circles),
        _ => Err(bad(cmd, "a plane `strategy` is points or circles")),
    }
}

fn count_param(p: &Value, key: &str, range: std::ops::RangeInclusive<u64>, default: u32, cmd: &str) -> Result<u32> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| range.contains(n))
            .map(|n| n as u32)
            .ok_or_else(|| bad(cmd, format!("`{key}` must be {} to {}", range.start(), range.end()))),
    }
}

fn mm_param(p: &Value, key: &str, default: f64, cmd: &str) -> Result<f64> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            v.as_f64().filter(|x| x.is_finite() && (0.0..=1000.0).contains(x)).ok_or_else(|| bad(cmd, format!("`{key}` must be mm from 0 to 1000")))
        }
    }
}

fn picks_param(p: &Value, cmd: &str) -> Result<Vec<[f64; 3]>> {
    match p.get("picks") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) if a.len() <= MAX_PICKS => a
            .iter()
            .map(|v| {
                serde_json::from_value::<[f64; 3]>(v.clone())
                    .ok()
                    .filter(|q| q.iter().all(|c| c.is_finite() && c.abs() < 1e9))
                    .ok_or_else(|| bad(cmd, "`picks` must be [[x, y, z], …]"))
            })
            .collect(),
        Some(_) => Err(bad(cmd, format!("`picks` must be a list of at most {MAX_PICKS} points"))),
    }
}

fn circle(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.circle";
    let points = count_param(p, "points", 3..=100, 4, cmd)?;
    let height = mm_param(p, "height", 2.0, cmd)?;
    let position = p.get("position").and_then(Value::as_bool).unwrap_or(false);
    let kind = CharacteristicKind::Circle { face: face(s, p, "face", 0, cmd)?, points, height, position };
    put(s, p, cmd, kind, 0.05, 100.0)
}

fn cylinder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.cylinder";
    let strategy = match p.get("strategy").and_then(Value::as_str) {
        None => CylinderProbing::Scan,
        Some("scan") => CylinderProbing::Scan,
        Some("circle_points") => CylinderProbing::CirclePoints,
        Some("points") => CylinderProbing::Points,
        Some(_) => return Err(bad(cmd, "`strategy` must be scan, circle_points or points")),
    };
    let kind = CharacteristicKind::Cylinder {
        face: face(s, p, "face", 0, cmd)?,
        strategy,
        paths: count_param(p, "paths", 1..=20, 2, cmd)?,
        points: count_param(p, "points", 3..=100, 7, cmd)?,
        height: mm_param(p, "height", 2.0, cmd)?,
        picks: picks_param(p, cmd)?,
    };
    put(s, p, cmd, kind, 0.05, 100.0)
}

fn plane(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.plane";
    let strategy = match p.get("strategy") {
        None | Some(Value::Null) => PlaneProbing::Points,
        Some(v) => plane_strategy(v, cmd)?,
    };
    let kind = CharacteristicKind::Plane {
        face: face(s, p, "face", 0, cmd)?,
        strategy,
        points: count_param(p, "points", 3..=100, 4, cmd)?,
        picks: picks_param(p, cmd)?,
    };
    put(s, p, cmd, kind, 0.05, 100.0)
}

fn probe_table(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.probe.table";
    let path = match p.get("rows") {
        None | Some(Value::Null) => insp::probe_table_path(),
        Some(Value::Array(a)) if a.len() <= insp::MAX_PROBE_ROWS => {
            let rows: Vec<insp::ProbeRow> =
                a.iter().map(|v| serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("a row: {e}")))).collect::<Result<_>>()?;
            Some(insp::save_probe_table(&rows).map_err(other)?)
        }
        Some(_) => return Err(bad(cmd, format!("`rows` must be a list of at most {} probes", insp::MAX_PROBE_ROWS))),
    };
    Ok(json!({"path": path.map(|p| p.display().to_string()), "rows": insp::probe_table()}))
}

fn distance(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.distance";
    let axis = match p.get("axis") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_str() {
            Some("x" | "X") => Some(Axis::X),
            Some("y" | "Y") => Some(Axis::Y),
            Some("z" | "Z") => Some(Axis::Z),
            Some("auto") => None,
            _ => return Err(bad(cmd, "`axis` must be x, y, z or auto")),
        },
    };
    let kind = CharacteristicKind::Distance { from: face(s, p, "from", 0, cmd)?, to: face(s, p, "to", 1, cmd)?, axis };
    put(s, p, cmd, kind, 0.05, 100.0)
}

fn angle(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.angle";
    let kind = CharacteristicKind::Angle { first: face(s, p, "first", 0, cmd)?, second: face(s, p, "second", 1, cmd)? };
    put(s, p, cmd, kind, 0.1, 90.0)
}

fn point(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.point";
    let points = match p.get("points") {
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= insp::MAX_POINTS => a.iter().map(|v| parse_face(v, cmd)).collect::<Result<Vec<_>>>()?,
        Some(Value::Array(a)) if a.len() > insp::MAX_POINTS => return Err(bad(cmd, format!("at most {} points", insp::MAX_POINTS))),
        Some(Value::Null) | None => vec![face(s, p, "face", 0, cmd)?],
        Some(_) => return Err(bad(cmd, "`points` must be a list of points {body, point}")),
    };
    let check = match p.get("check").and_then(Value::as_str) {
        None => insp::PointCheck::Normal,
        Some("normal") => insp::PointCheck::Normal,
        Some("coordinates") => insp::PointCheck::Coordinates,
        Some(_) => return Err(bad(cmd, "`check` must be normal or coordinates")),
    };
    let kind = CharacteristicKind::Point { points, check };
    put(s, p, cmd, kind, 0.05, 100.0)
}

// ------------------------------------------------------------------------------------------
// Export
// ------------------------------------------------------------------------------------------

/// The neutral program of the setup `setup` (default: the active one) through its NC program
/// `nc` (default: its first one; a setup without NC programs uses its own post settings), and
/// the setup as that NC program sees it.
fn setup_program(s: &Session, p: &Value, cmd: &str) -> Result<(Setup, insp::NcProgram, solidcraft_inspection::program::Program)> {
    let given_nc = id_param(p, "nc", cmd)?;
    let id = match given_nc {
        Some(n) if p.get("setup").is_none() => s
            .doc
            .cmm
            .setups
            .iter()
            .find(|st| st.programs.iter().any(|x| x.id == n))
            .map(|st| st.id)
            .ok_or_else(|| bad(cmd, format!("there is no NC program {n}")))?,
        _ => setup_id(s, p, cmd)?,
    };
    let setup = s.doc.cmm.setup(id).cloned().ok_or_else(|| bad(cmd, "the setup is gone"))?;
    let model = s.world_state();
    let nc = match given_nc {
        Some(n) => setup.programs.iter().find(|x| x.id == n).cloned().ok_or_else(|| bad(cmd, format!("setup {id} has no NC program {n}")))?,
        None => insp::default_program(&setup),
    };
    let name = insp::plan_name(&setup, &nc, s.path.as_deref());
    let out = insp::for_program(&model, &setup, &nc);
    let program = insp::program(&model, &out, &nc, &name).map_err(other)?;
    Ok((setup, nc, program))
}

fn program(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.program";
    let (_, _, program) = setup_program(s, p, cmd)?;
    let v = serde_json::to_value(&program).map_err(other)?;
    match p.get("path") {
        None | Some(Value::Null) => {}
        Some(Value::String(path)) if !path.trim().is_empty() && path.len() < 4096 => {
            let text = serde_json::to_string_pretty(&v).map_err(other)?;
            std::fs::write(path.trim(), text).map_err(|e| other(format!("{path}: {e}")))?;
        }
        Some(_) => return Err(bad(cmd, "`path` must be a file path")),
    }
    Ok(v)
}

/// How long a postprocessor may take.
const POST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Runs a postprocessor: `post <program.json> <folder> [--replace]`. It writes the plan into
/// `<folder>` and prints one JSON object (`folder`, `skipped`, `warnings`) on success; on
/// failure it exits non-zero with the reason on stderr.
fn run_post(post: &std::path::Path, program: &std::path::Path, dir: &std::path::Path, replace: bool) -> Result<Value> {
    let mut args = vec![program.as_os_str().to_owned(), dir.as_os_str().to_owned()];
    if replace {
        args.push("--replace".into());
    }
    insp::run_post_with(post, &args, POST_TIMEOUT).map_err(other)
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.export";
    let (setup, nc, program) = setup_program(s, p, cmd)?;
    let post = insp::post_path(&nc).ok_or_else(|| {
        other(tr!(
            "no postprocessor found: put the post's program into {folder}",
            folder = insp::data_folder().map(|d| d.join("post").display().to_string()).unwrap_or_else(|| "<data>/inspection/post".into())
        ))
    })?;
    let dir = match p.get("dir") {
        Some(Value::String(d)) if !d.trim().is_empty() && d.len() < 4096 => std::path::PathBuf::from(d.trim()),
        Some(Value::String(_)) | None | Some(Value::Null) => insp::output_dir(&nc).ok_or_else(|| other(tr!("no output folder")))?,
        Some(_) => return Err(bad(cmd, "`dir` must be a folder path")),
    };
    std::fs::create_dir_all(&dir).map_err(|e| other(format!("{}: {e}", dir.display())))?;
    // the neutral program stays beside the plan: what was handed to the post
    let program_file = dir.join(format!("{}.cmm-program.json", program.plan));
    let text = serde_json::to_string_pretty(&program).map_err(other)?;
    std::fs::write(&program_file, text).map_err(|e| other(format!("{}: {e}", program_file.display())))?;
    let replace = p.get("replace").and_then(Value::as_bool) == Some(true);
    let answer = run_post(&post, &program_file, &dir, replace)?;
    let folder = answer.get("folder").and_then(Value::as_str).unwrap_or_default().to_string();
    s.log.push(tr!("Inspection plan written ({post}): {folder}", post = insp::post_name(&post), folder = folder));
    // the program, the CAD model and the setup sheet go into the plan's folder (the one the
    // measuring software opens), else beside it
    let plan_dir = Some(std::path::PathBuf::from(&folder)).filter(|f| !folder.is_empty() && f.is_dir()).unwrap_or_else(|| dir.clone());
    let program_file = {
        let moved = plan_dir.join(format!("{}.cmm-program.json", program.plan));
        if moved != program_file && std::fs::rename(&program_file, &moved).is_ok() { moved } else { program_file }
    };
    // the CAD model in the plan's base system for the CAD import (a failure is only a warning)
    let mut warnings: Vec<Value> = answer.get("warnings").and_then(Value::as_array).cloned().unwrap_or_default();
    let step_file = plan_dir.join(format!("{}.step", program.plan));
    let step = if nc.no_step {
        None
    } else {
        match insp::base_system_step(&s.world_state(), &setup, &program).and_then(|b| std::fs::write(&step_file, b).map_err(|e| e.to_string())) {
            Ok(()) => Some(step_file.display().to_string()),
            Err(e) => {
                warnings.push(json!(tr!("CAD model {file}: {e}", file = step_file.display(), e = e)));
                None
            }
        }
    };
    // the setup sheet for the technician beside the plan (a failure here is only a warning)
    let sheet_file = plan_dir.join(sheet_name(&program.plan));
    let sheet = match (!nc.no_sheet).then(|| crate::inspection_sheet::setup_sheet(s, &setup, &nc, &program)) {
        None => None,
        Some(Ok(pdf)) => match std::fs::write(&sheet_file, pdf) {
            Ok(()) => Some(sheet_file.display().to_string()),
            Err(e) => {
                warnings.push(json!(tr!("Setup sheet {file}: {e}", file = sheet_file.display(), e = e)));
                None
            }
        },
        Some(Err(e)) => {
            warnings.push(json!(tr!("Setup sheet: {e}", e = e)));
            None
        }
    };
    Ok(json!({
        "setup": setup.id,
        "nc": (nc.id != 0).then_some(nc.id),
        "folder": folder,
        "plan": program.plan,
        "program": program_file.display().to_string(),
        "post": insp::post_name(&post),
        "skipped": answer.get("skipped").cloned().unwrap_or_else(|| json!([])),
        "warnings": warnings,
        "sheet": sheet,
        "step": step,
    }))
}

/// Reads the measurement results of a plan back through its postprocessor and shows them.
fn results(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.results";
    match p.get("clear") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => {
            s.cmm_results = None;
            return Ok(json!({"cleared": true}));
        }
        Some(_) => return Err(bad(cmd, "`clear` must be true or false")),
    }
    let (setup, nc, program) = setup_program(s, p, cmd)?;
    let folder = match p.get("path") {
        Some(Value::String(d)) if !d.trim().is_empty() && d.len() < 4096 => std::path::PathBuf::from(d.trim()),
        None | Some(Value::Null) => insp::output_dir(&nc).ok_or_else(|| other(tr!("no output folder")))?.join(&program.plan),
        Some(_) => return Err(bad(cmd, "`path` must be a folder path")),
    };
    if !folder.is_dir() {
        return Err(other(tr!("no inspection plan folder {folder} (create the plan and measure first)", folder = folder.display())));
    }
    let post = insp::post_path(&nc).ok_or_else(|| other(tr!("no postprocessor found")))?;
    let answer = insp::run_post_with(&post, &["--results".into(), folder.as_os_str().to_owned()], POST_TIMEOUT).map_err(other)?;
    let read = crate::inspection_results::parse(answer).map_err(other)?;
    // the program the plan was made from, as it lies in the plan's folder (its names and nominals
    // are what was measured, whatever changed in the design or the language since), else today's
    let measured = std::fs::read_to_string(folder.join(format!("{}.cmm-program.json", program.plan)))
        .ok()
        .and_then(|t| serde_json::from_str::<solidcraft_inspection::program::Program>(&t).ok())
        .filter(|p| p.validate().is_ok());
    let points = crate::inspection_results::place(measured.as_ref().unwrap_or(&program), &read).map_err(other)?;
    let loaded = crate::inspection_results::LoadedResults {
        setup: setup.id,
        nc: (nc.id != 0).then_some(nc.id),
        plan: program.plan.clone(),
        folder: folder.display().to_string(),
        measured: read.measured,
        points,
        characteristics: read.characteristics,
    };
    let v = loaded.describe();
    let (inside, out) = loaded.counts();
    s.log.push(tr!(
        "Measurement results {plan}: {inside} characteristics in tolerance, {out} out of tolerance",
        plan = program.plan,
        inside = inside,
        out = out
    ));
    s.cmm_results = Some(loaded);
    Ok(v)
}

// ------------------------------------------------------------------------------------------
// NC programs
// ------------------------------------------------------------------------------------------

/// Creates or edits an NC program; with `export`, writes its plan right after.
fn nc_program(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.nc";
    let editing = id_param(p, "nc", cmd)?;
    let sid = match editing {
        Some(n) => s
            .doc
            .cmm
            .setups
            .iter()
            .find(|st| st.programs.iter().any(|x| x.id == n))
            .map(|st| st.id)
            .ok_or_else(|| bad(cmd, format!("there is no NC program {n}")))?,
        None => setup_id(s, p, cmd)?,
    };
    let name = text_param(p, "name", cmd)?;
    let plan_name = text_param(p, "plan_name", cmd)?;
    let flag = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(bad(cmd, format!("`{k}` must be true or false"))),
    };
    let (sheet, step) = (flag("sheet")?, flag("step")?);
    let (post, out) = (path_param(p, "post", cmd)?, path_param(p, "output_dir", cmd)?);
    let options = match p.get("options") {
        None => None,
        Some(Value::Null) => Some(std::collections::BTreeMap::new()),
        Some(Value::Object(m)) if m.len() <= 100 => Some(
            m.iter()
                .map(|(k, v)| match v {
                    Value::String(t) if k.len() <= 100 && t.len() <= 4096 => Ok((k.clone(), t.clone())),
                    Value::Bool(b) if k.len() <= 100 => Ok((k.clone(), if *b { "on" } else { "off" }.to_string())),
                    _ => Err(bad(cmd, "`options` values must be texts or true/false")),
                })
                .collect::<Result<std::collections::BTreeMap<_, _>>>()?,
        ),
        Some(_) => return Err(bad(cmd, "`options` must be {key: text}")),
    };
    let st = s.doc.cmm.setup(sid).ok_or_else(|| bad(cmd, "the setup is gone"))?;
    let chars = match p.get("characteristics") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::Array(a)) if a.len() <= MAX_CHARACTERISTICS => {
            let mut ids = Vec::new();
            for v in a {
                let id = v.as_u64().ok_or_else(|| bad(cmd, "`characteristics` must be ids"))?;
                if !st.characteristics.iter().any(|c| c.id == id) {
                    return Err(bad(cmd, format!("setup {sid} has no characteristic {id}")));
                }
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            Some(Some(ids))
        }
        Some(_) => return Err(bad(cmd, "`characteristics` must be a list of ids or null (all)")),
    };
    if editing.is_none() && st.programs.len() >= insp::MAX_NC_PROGRAMS {
        return Err(bad(cmd, format!("at most {} NC programs in a setup", insp::MAX_NC_PROGRAMS)));
    }
    let new_id = s.doc.cmm.next_id();
    let plans = &mut s.doc_mut().cmm;
    let Some(st) = plans.setup_mut(sid) else { return Err(bad(cmd, "the setup is gone")) };
    let id = match editing {
        Some(n) => n,
        None => {
            let name = st.next_program_name(tr!("NC Program"));
            st.programs.push(insp::NcProgram::new(new_id, name));
            new_id
        }
    };
    if let Some(nc) = st.programs.iter_mut().find(|x| x.id == id) {
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            nc.name = n;
        }
        if let Some(n) = plan_name {
            nc.plan_name = n;
        }
        if let Some(t) = post {
            nc.post = t;
        }
        if let Some(t) = out {
            nc.output_dir = t;
        }
        if let Some(o) = options {
            nc.options = o;
        }
        if let Some(c) = chars {
            nc.characteristics = c;
        }
        if let Some(b) = sheet {
            nc.no_sheet = !b;
        }
        if let Some(b) = step {
            nc.no_step = !b;
        }
    }
    plans.active = Some(sid);
    let exported = if p.get("export").and_then(Value::as_bool) == Some(true) {
        Some(export(s, &json!({"setup": sid, "nc": id, "replace": true, "dir": p.get("dir").cloned().unwrap_or(Value::Null)}))?)
    } else {
        None
    };
    reply(s, sid, json!({"nc": id, "export": exported}))
}

fn delete_nc(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.nc.delete";
    let n = id_param(p, "nc", cmd)?.ok_or_else(|| bad(cmd, "`nc` is required"))?;
    let sid = s
        .doc
        .cmm
        .setups
        .iter()
        .find(|st| st.programs.iter().any(|x| x.id == n))
        .map(|st| st.id)
        .ok_or_else(|| bad(cmd, format!("there is no NC program {n}")))?;
    if let Some(st) = s.doc_mut().cmm.setup_mut(sid) {
        st.programs.retain(|x| x.id != n);
    }
    reply(s, sid, json!({"deleted": n}))
}

/// The setup sheet's file name: `<plan>_setup-sheet.pdf` (German: `<plan>_Einrichtblatt.pdf`).
fn sheet_name(plan: &str) -> String {
    tr!("{plan}_setup-sheet.pdf", plan = plan)
}

/// The setup sheet (PDF) of an NC program, without running the post.
fn sheet(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.sheet";
    let (setup, nc, program) = setup_program(s, p, cmd)?;
    let pdf = crate::inspection_sheet::setup_sheet(s, &setup, &nc, &program).map_err(other)?;
    let path = match p.get("path") {
        Some(Value::String(t)) if !t.trim().is_empty() && t.len() < 4096 => std::path::PathBuf::from(t.trim()),
        None | Some(Value::Null) => {
            // into the plan's folder once it is written, else the output folder
            let out = insp::output_dir(&nc).ok_or_else(|| other(tr!("no output folder")))?;
            let plan_dir = out.join(&program.plan);
            if plan_dir.is_dir() { plan_dir } else { out }.join(sheet_name(&program.plan))
        }
        Some(_) => return Err(bad(cmd, "`path` must be a file path")),
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| other(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&path, &pdf).map_err(|e| other(format!("{}: {e}", path.display())))?;
    Ok(json!({"path": path.display().to_string(), "bytes": pdf.len()}))
}

/// The settings a postprocessor offers (`post --describe`).
fn post_options(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cmm.post.options";
    let given = path_param(p, "post", cmd)?.flatten();
    let probe = insp::NcProgram { post: given, ..Default::default() };
    let Some(post) = insp::post_path(&probe) else { return Ok(json!({"post": null, "options": []})) };
    let options = insp::post_settings(&post);
    Ok(json!({"post": insp::post_name(&post), "options": options}))
}

#[cfg(test)]
#[path = "cmm_tests.rs"]
mod tests;
