//! CMM dialogs: the setup's probe system (machine and probe configuration), the
//! alignment, one dialog per measurement or characteristic (circle, cylinder, plane, distance,
//! angle, point) and the probe table. Each runs its `cmm.*` command; opened from the tree they
//! edit the setup or characteristic in place (`extra`: `setup` / `characteristic`). With the
//! strategy "points" a plane or cylinder dialog takes points clicked on the face.

use serde_json::{Map, Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::doc::cmm::{
    AlignBores, Alignment, Characteristic, CharacteristicKind, CylinderProbing, DirectionRef, FaceRef, OriginRef, PlaneProbing, PointCheck,
    SafetyPlane,
};
use solidcraft_engine::inspection as insp;

use crate::SolveApp;
use crate::dialogs::{Kind, face_sel};
use crate::selection::{FACES, PLANAR_FACES, SelInput};
use crate::ui_kit::{self, Text, choice_row, hint_row, text_row};

/// One setting of a postprocessor, as it describes it, with the value in the dialog.
#[derive(Clone, Debug)]
pub struct NcSetting {
    pub key: String,
    pub label: String,
    /// A switch (`on`/`off`), else a text.
    pub flag: bool,
    pub on: bool,
    pub text: String,
}

#[derive(Clone, Debug)]
pub enum Km {
    /// New Setup / Probe System: the probe, the safety plane and how planes are probed (the
    /// postprocessor and its settings belong to the setup's NC programs).
    Setup {
        name: String,
        probe: usize,
        probes: Vec<String>,
        safety: usize,
        plane_probing: usize,
    },
    /// An NC program: its postprocessor with the post's own settings, plan name, output folder
    /// and the characteristics it outputs. OK saves it and writes the plan.
    Nc {
        name: String,
        /// 0: the first post in the post folder.
        post: usize,
        posts: Vec<String>,
        plan_name: String,
        output_dir: String,
        /// The settings of each post (`posts` order; index 0: the first post's), as they
        /// describe them, with this program's values.
        settings: Vec<Vec<NcSetting>>,
        /// The setup's characteristics: (id, label, output).
        chars: Vec<(u64, String, bool)>,
        /// Write the setup sheet (PDF) beside the plan.
        sheet: bool,
        /// Write the CAD model (STEP in the base system) beside the plan.
        step: bool,
    },
    /// The alignment; `bores`: 0 cylinder, 1 circle; `clamp`: the primary face is the clamping
    /// face (the plate beside the part is probed), with the points' distance from the part and
    /// the points themselves (`X Y; X Y; …`, empty: automatic).
    Alignment {
        bores: usize,
        clamp: bool,
        margin: String,
        points: String,
    },
    /// A circle in a bore: points, its height below the opening plane, its position.
    Circle {
        name: String,
        tol: String,
        points: String,
        height: String,
        position: bool,
        safety: usize,
    },
    /// A bore as a cylinder, with its strategy.
    Cylinder {
        name: String,
        tol: String,
        strategy: usize,
        paths: String,
        points: String,
        height: String,
        safety: usize,
    },
    /// How a plane is probed.
    Plane {
        name: String,
        strategy: usize,
        points: String,
        safety: usize,
    },
    Distance {
        name: String,
        tol: String,
        axis: usize,
        safety: usize,
    },
    Angle {
        name: String,
        tol: String,
        safety: usize,
    },
    /// A point group: one or many points clicked on the part (e.g. along a contour); `check`:
    /// 0 deviation along the normal, 1 coordinates.
    Point {
        name: String,
        tol: String,
        safety: usize,
        check: usize,
    },
    /// The probe table: the dimensions of each probe, one shown at a time; probes are added and
    /// removed here (the table is the only source of probes).
    ProbeTable {
        rows: Vec<ProbeText>,
        shown: usize,
        /// The name of a probe to add (as on the machine, e.g. its configuration's name).
        new_name: String,
    },
}

/// A probe table row as the dialog edits it (numbers as text).
#[derive(Clone, Debug, Default)]
pub struct ProbeText {
    pub file: String,
    /// Ball Ø, length, measuring length, shaft Ø, 2nd measuring length, 2nd shaft Ø, base body Ø.
    pub dims: [String; 7],
    pub order_no: String,
    pub note: String,
}

/// The probe dimensions: label and what it means.
pub const PROBE_DIMS: [(&str, &str); 7] = [
    ("Ball Ø", "mm"),
    ("Probe Length", "mm"),
    ("Measuring Length", "mm"),
    ("Shaft Ø", "mm"),
    ("2nd Measuring Length", "mm"),
    ("2nd Shaft Ø", "mm"),
    ("Base Body Ø", "mm"),
];

const AXES: [&str; 4] = ["Automatic", "X", "Y", "Z"];
const CYLINDER_STRATEGIES: [&str; 3] = ["Scan circular paths", "Circular paths of points", "Points (pick on the wall)"];
const PLANE_STRATEGIES: [&str; 2] = ["Points", "Circular paths"];
const PLANE_DEFAULTS: [&str; 2] = ["4 points", "Circular paths"];
/// How the alignment's bores are measured (index 0: cylinder, 1: circle).
const ALIGN_BORES: [&str; 2] = ["Cylinder", "Circle"];
/// The safety planes of a characteristic: the setup's, then [`SafetyPlane::ALL`].
const CHAR_SAFETY: [&str; 8] = ["From the setup", "+Z", "Automatic (face direction)", "+X", "-X", "+Y", "-Y", "-Z"];
const SETUP_SAFETY: [&str; 7] = ["+Z", "Automatic (face direction)", "+X", "-X", "+Y", "-Y", "-Z"];

fn safety_index(s: Option<SafetyPlane>) -> usize {
    s.and_then(|s| SafetyPlane::ALL.iter().position(|x| *x == s)).map_or(0, |i| i + 1)
}

fn safety_json(i: usize) -> Value {
    i.checked_sub(1).and_then(|i| SafetyPlane::ALL.get(i)).map_or(Value::Null, |s| serde_json::to_value(s).unwrap_or(Value::Null))
}

fn safety_of_index(i: usize) -> Option<SafetyPlane> {
    i.checked_sub(1).and_then(|i| SafetyPlane::ALL.get(i)).copied()
}

impl Km {
    pub fn title(&self) -> &'static str {
        match self {
            Km::Setup { .. } => "SETUP – PROBE SYSTEM",
            Km::Nc { .. } => "NC PROGRAM",
            Km::Alignment { .. } => "ALIGNMENT",
            Km::Circle { .. } => "CIRCLE",
            Km::Cylinder { .. } => "CYLINDER MEASUREMENT",
            Km::Plane { .. } => "PLANE MEASUREMENT",
            Km::Distance { .. } => "DISTANCE",
            Km::Angle { .. } => "ANGLE",
            Km::Point { .. } => "POINT",
            Km::ProbeTable { .. } => "PROBE TABLE",
        }
    }

    pub fn command(&self) -> &'static str {
        match self {
            Km::Setup { .. } => "cmm.setup",
            Km::Nc { .. } => "cmm.nc",
            Km::Alignment { .. } => "cmm.alignment",
            Km::Circle { .. } => "cmm.circle",
            Km::Cylinder { .. } => "cmm.cylinder",
            Km::Plane { .. } => "cmm.plane",
            Km::Distance { .. } => "cmm.distance",
            Km::Angle { .. } => "cmm.angle",
            Km::Point { .. } => "cmm.point",
            Km::ProbeTable { .. } => "cmm.probe.table",
        }
    }

    /// Whether the dialog picks single points on its face (input 1).
    fn picks_points(&self) -> bool {
        matches!(self, Km::Cylinder { strategy: 2, .. } | Km::Plane { strategy: 0, .. })
    }
}

/// The input that takes points picked on the face.
fn points_input() -> SelInput {
    SelInput::new("Points", FACES, true).places()
}

/// Adds the points input when the strategy picks points, takes it away otherwise.
fn sync_points(k: &Km, inputs: &mut Vec<SelInput>, active: &mut usize) {
    let has = inputs.get(1).is_some_and(|i| i.places);
    if k.picks_points() && !has && inputs.len() == 1 {
        inputs.push(points_input());
        // a face already picked: the next clicks place points
        if inputs.first().is_some_and(|i| !i.items.is_empty()) {
            *active = 1;
        }
    } else if !k.picks_points() && has {
        inputs.truncate(1);
        *active = 0;
    }
}

fn tol_text(t: f64) -> String {
    let s = format!("{t:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn path_label(p: &str) -> String {
    let path = std::path::Path::new(p);
    let file = path.file_name().and_then(|f| f.to_str()).unwrap_or(p);
    match path.parent().and_then(|d| d.file_name()).and_then(|d| d.to_str()) {
        Some(dir) if file == "inspection" => format!("{dir}/{file}"),
        _ => insp::probe_key(file),
    }
}

fn setup_kind(app: &SolveApp, setup: Option<u64>) -> Km {
    let probes: Vec<String> = insp::probe_choices();
    let plans = &app.session.doc.cmm;
    let st = setup.and_then(|id| plans.setup(id));
    let pos = |list: &[String], v: Option<&String>| v.and_then(|v| list.iter().position(|x| x == v)).map_or(0, |i| i + 1);
    Km::Setup {
        name: st.map_or_else(|| plans.next_setup_name(), |s| s.name.clone()),
        probe: pos(&probes, st.and_then(|s| s.probe.probe_file.as_ref())),
        probes,
        safety: st.map_or(0, |s| safety_index(Some(s.safety_plane)).saturating_sub(1)),
        plane_probing: st.map_or(0, |s| usize::from(s.plane_probing == PlaneProbing::Circles)),
    }
}

/// The NC program dialog: program `nc` of setup `setup` (none: a new one with the defaults).
/// Each post is asked once for its settings.
fn nc_kind(app: &SolveApp, setup: u64, nc: Option<u64>) -> Km {
    let posts: Vec<String> = insp::posts().iter().map(|p| insp::post_name(p)).collect();
    let st = app.session.doc.cmm.setup(setup);
    let fresh = st.map(|s| insp::NcProgram::new(0, s.next_program_name(solidcraft_engine::tr!("NC Program")))).unwrap_or_default();
    let prog = nc.and_then(|n| st.and_then(|s| s.programs.iter().find(|x| x.id == n))).cloned().unwrap_or(fresh);
    let post = prog.post.as_ref().and_then(|v| posts.iter().position(|x| x == v)).map_or(0, |i| i + 1);
    // index 0: the first post (what "automatic" uses), then each post
    let mut paths: Vec<Option<std::path::PathBuf>> = vec![insp::posts().into_iter().next()];
    paths.extend(insp::posts().into_iter().map(Some));
    let settings = paths
        .iter()
        .map(|p| {
            p.as_ref()
                .map(|p| insp::post_settings(p))
                .unwrap_or_default()
                .iter()
                .map(|o| {
                    let key = o["key"].as_str().unwrap_or_default().to_string();
                    let default = o["default"].as_str().unwrap_or_default();
                    let value = prog.options.get(&key).map_or(default, String::as_str).to_string();
                    let flag = o["kind"] == "bool";
                    let on = !matches!(value.trim().to_ascii_lowercase().as_str(), "off" | "false" | "nein" | "aus" | "0" | "");
                    NcSetting { key, label: o["label"].as_str().unwrap_or_default().to_string(), flag, on, text: value }
                })
                .collect()
        })
        .collect();
    let chars = st
        .map(|s| {
            let model = app.session.world_state();
            insp::in_alignment(&model, &s.alignment, || {
                s.characteristics.iter().map(|c| (c.id, insp::characteristic_name(&model, c), prog.outputs(c.id))).collect()
            })
        })
        .unwrap_or_default();
    Km::Nc {
        name: prog.name.clone(),
        post,
        posts,
        plan_name: prog.plan_name.clone(),
        output_dir: prog.output_dir.clone().unwrap_or_default(),
        settings,
        chars,
        sheet: !prog.no_sheet,
        step: !prog.no_step,
    }
}

/// Opens the dialog of NC program `nc` of `setup` (none: a new one).
pub fn edit_nc(app: &mut SolveApp, setup: u64, nc: Option<u64>) {
    if app.session.doc.cmm.active != Some(setup) {
        let _ = app.run("cmm.setup.activate", json!({"setup": setup}));
    }
    let km = nc_kind(app, setup, nc);
    let mut extra = Map::new();
    extra.insert("setup".into(), json!(setup));
    if let Some(n) = nc {
        extra.insert("nc".into(), json!(n));
    }
    app.start("cmm.nc");
    if let Some(d) = app.dialog.as_mut()
        && matches!(d.kind, Kind::Cmm(_))
    {
        d.kind = Kind::Cmm(km);
        d.inputs = Vec::new();
        d.extra = extra;
    }
}

fn probe_table_kind() -> Km {
    let n = |x: Option<f64>| x.map(tol_text).unwrap_or_default();
    let rows = insp::probe_table()
        .into_iter()
        .map(|r| ProbeText {
            dims: [n(r.ball), n(r.length), n(r.measuring_length), n(r.shaft), n(r.measuring_length2), n(r.shaft2), n(r.body)],
            file: r.file,
            order_no: r.order_no,
            note: r.note,
        })
        .collect();
    Km::ProbeTable { rows, shown: 0, new_name: String::new() }
}

fn input(label: &'static str, accept: crate::selection::Accept, multi: bool) -> SelInput {
    SelInput::new(label, accept, multi)
}

fn alignment_inputs() -> Vec<SelInput> {
    vec![input("Primary Plane", PLANAR_FACES, false), input("Direction", FACES, true), input("Origin X", FACES, true), input("Origin Y", FACES, true)]
}

fn characteristic_kind(id: &str) -> Option<(Km, Vec<SelInput>)> {
    let tol = || "0.05".to_string();
    let name = String::new;
    Some(match id {
        "cmm.circle" => (
            Km::Circle { name: name(), tol: tol(), points: "4".into(), height: "2".into(), position: true, safety: 0 },
            vec![input("Bore", FACES, false)],
        ),
        "cmm.cylinder" => (
            Km::Cylinder { name: name(), tol: tol(), strategy: 0, paths: "2".into(), points: "7".into(), height: "2".into(), safety: 0 },
            vec![input("Bore", FACES, false)],
        ),
        "cmm.plane" => {
            (Km::Plane { name: name(), strategy: 0, points: "4".into(), safety: 0 }, vec![input("Plane", PLANAR_FACES, false), points_input()])
        }
        "cmm.distance" => {
            (Km::Distance { name: name(), tol: tol(), axis: 0, safety: 0 }, vec![input("Element 1", FACES, false), input("Element 2", FACES, false)])
        }
        "cmm.angle" => (
            Km::Angle { name: name(), tol: "0.1".into(), safety: 0 },
            vec![input("Plane 1", PLANAR_FACES, false), input("Plane 2", PLANAR_FACES, false)],
        ),
        "cmm.point" => (Km::Point { name: name(), tol: tol(), safety: 0, check: 0 }, vec![input("Points", FACES, true).places()]),
        _ => return None,
    })
}

/// The dialog for a CMM command.
pub fn start(app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    let (km, inputs) = match id {
        "cmm.setup" => (setup_kind(app, None), Vec::new()),
        "cmm.probe.table" => (probe_table_kind(), Vec::new()),
        "cmm.nc" => {
            let setup = app.session.doc.cmm.active_setup()?;
            (nc_kind(app, setup.id, setup.programs.first().map(|p| p.id)), Vec::new())
        }
        "cmm.alignment" => {
            let mut inputs = alignment_inputs();
            let mut km = alignment_kind(&Alignment::default());
            if let Some(st) = app.session.doc.cmm.active_setup() {
                fill_alignment(app, &mut inputs, &st.alignment);
                km = alignment_kind(&st.alignment);
            }
            (km, inputs)
        }
        other => characteristic_kind(other)?,
    };
    Some((Kind::Cmm(km), inputs))
}

/// The alignment dialog's fields for alignment `a`.
fn alignment_kind(a: &Alignment) -> Km {
    let (clamp, margin, points) = match &a.clamp {
        Some(c) => {
            (true, insp::num(c.margin), c.points.iter().map(|q| format!("{} {}", insp::num(q[0]), insp::num(q[1]))).collect::<Vec<_>>().join("; "))
        }
        None => (false, String::new(), String::new()),
    };
    Km::Alignment { bores: usize::from(a.bores == AlignBores::Circle), clamp, margin, points }
}

/// The clamping plate the alignment dialog's fields give (`None`: not clamped); fields that
/// do not read as numbers fall back to the automatic points.
pub fn clamp_of(km: &Km) -> Option<insp::ClampPlane> {
    let Km::Alignment { clamp: true, margin, points, .. } = km else { return None };
    let mut c = insp::ClampPlane::default();
    if let Some(m) = number(margin).filter(|m| *m > 0.0) {
        c.margin = m;
    }
    c.points = parse_points(points).unwrap_or_default();
    Some(c)
}

/// `X Y; X Y; …` (a decimal comma is fine) as points; `None` when a pair is not two numbers.
fn parse_points(text: &str) -> Option<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    for pair in text.split(';').map(str::trim).filter(|p| !p.is_empty()).take(insp::MAX_CLAMP_POINTS) {
        let v: Vec<f64> = pair.split_whitespace().filter_map(number).collect();
        match v.as_slice() {
            [x, y] => out.push([*x, *y]),
            _ => return None,
        }
    }
    Some(out)
}

fn sel_of(app: &SolveApp, f: &FaceRef) -> Sel {
    let p = solidcraft_engine::geom::Vec3::new(f.point[0], f.point[1], f.point[2]);
    face_sel(&app.session, p).unwrap_or(Sel::Face { body: f.body.clone(), index: 0, point: p })
}

fn fill_alignment(app: &SolveApp, inputs: &mut [SelInput], a: &Alignment) {
    fn origin(o: &Option<OriginRef>) -> Vec<&FaceRef> {
        match o {
            Some(OriginRef::Side { face } | OriginRef::Bore { face }) => vec![face],
            Some(OriginRef::Middle { first, second }) => vec![first, second],
            None => Vec::new(),
        }
    }
    let direction: Vec<&FaceRef> = match &a.direction {
        Some(DirectionRef::Side { face }) => vec![face],
        Some(DirectionRef::Bores { first, second }) => vec![first, second],
        None => Vec::new(),
    };
    let same_bore = matches!((&a.origin_x, &a.origin_y), (Some(OriginRef::Bore { face: x }), Some(OriginRef::Bore { face: y })) if x == y);
    let lists: [Vec<&FaceRef>; 4] =
        [a.primary.iter().collect(), direction, origin(&a.origin_x), if same_bore { Vec::new() } else { origin(&a.origin_y) }];
    for (i, list) in inputs.iter_mut().zip(lists) {
        i.items = list.into_iter().map(|f| sel_of(app, f)).collect();
    }
}

/// Opens the dialog of a setup (`what`: "setup" for its probe system, "alignment") or of a
/// characteristic, filled from it, to edit it.
/// Opens a new measurement (`cmd`: `cmm.plane`, `cmm.cylinder` or `cmm.circle`) of `face`, with
/// the face already chosen: an alignment element gets its own strategy this way.
pub fn measure_face(app: &mut SolveApp, setup: u64, cmd: &str, face: &FaceRef) {
    if app.session.doc.cmm.active != Some(setup) {
        let _ = app.run("cmm.setup.activate", json!({"setup": setup}));
    }
    let Some((km, mut inputs)) = characteristic_kind(cmd) else { return };
    if let Some(first) = inputs.first_mut() {
        first.items = vec![sel_of(app, face)];
    }
    let mut active = 0;
    sync_points(&km, &mut inputs, &mut active);
    app.start(cmd);
    if let Some(d) = app.dialog.as_mut()
        && matches!(d.kind, Kind::Cmm(_))
    {
        d.kind = Kind::Cmm(km);
        d.inputs = inputs;
        d.active = active;
    }
}

pub fn edit(app: &mut SolveApp, setup: u64, what: &str, characteristic: Option<u64>) {
    let Some(st) = app.session.doc.cmm.setup(setup).cloned() else { return };
    if app.session.doc.cmm.active != Some(setup) {
        let _ = app.run("cmm.setup.activate", json!({"setup": setup}));
    }
    let mut extra = Map::new();
    let (cmd, km, inputs) = match (what, characteristic.and_then(|c| st.characteristics.iter().find(|x| x.id == c))) {
        ("setup", _) => {
            extra.insert("setup".into(), json!(setup));
            ("cmm.setup", setup_kind(app, Some(setup)), Vec::new())
        }
        ("alignment", _) => {
            let mut inputs = alignment_inputs();
            fill_alignment(app, &mut inputs, &st.alignment);
            ("cmm.alignment", alignment_kind(&st.alignment), inputs)
        }
        (_, Some(c)) => {
            extra.insert("characteristic".into(), json!(c.id));
            extra.insert("probe_file".into(), json!(c.probe_file));
            let Some((mut km, mut inputs)) = characteristic_kind(c.kind.command()) else { return };
            let tol = tol_text(c.tol);
            let sp = safety_index(c.safety_plane);
            let mut picks: &[[f64; 3]] = &[];
            match (&mut km, &c.kind) {
                (
                    Km::Circle { name, tol: t, points: n, height: h, position: p, safety },
                    CharacteristicKind::Circle { points, height, position, .. },
                ) => {
                    (*name, *t, *n, *h, *p, *safety) = (c.name.clone(), tol, points.to_string(), tol_text(*height), *position, sp);
                }
                (
                    Km::Cylinder { name, tol: t, strategy: st, paths: pa, points: n, height: h, safety },
                    CharacteristicKind::Cylinder { strategy, paths, points, height, picks: pk, .. },
                ) => {
                    *st = match strategy {
                        CylinderProbing::Scan => 0,
                        CylinderProbing::CirclePoints => 1,
                        CylinderProbing::Points => 2,
                    };
                    (*name, *t, *pa, *n, *h, *safety) = (c.name.clone(), tol, paths.to_string(), points.to_string(), tol_text(*height), sp);
                    picks = pk;
                }
                (Km::Plane { name, strategy: st, points: n, safety }, CharacteristicKind::Plane { strategy, points, picks: pk, .. }) => {
                    *st = usize::from(*strategy == PlaneProbing::Circles);
                    (*name, *n, *safety) = (c.name.clone(), points.to_string(), sp);
                    picks = pk;
                }
                (Km::Distance { name, tol: t, axis: a, safety }, CharacteristicKind::Distance { axis, .. }) => {
                    (*name, *t, *safety) = (c.name.clone(), tol, sp);
                    *a = match axis {
                        None => 0,
                        Some(solidcraft_engine::doc::cmm::Axis::X) => 1,
                        Some(solidcraft_engine::doc::cmm::Axis::Y) => 2,
                        Some(solidcraft_engine::doc::cmm::Axis::Z) => 3,
                    };
                }
                (Km::Point { name, tol: t, safety, check: k }, CharacteristicKind::Point { check, .. }) => {
                    (*name, *t, *safety) = (c.name.clone(), tol, sp);
                    *k = usize::from(*check == PointCheck::Coordinates);
                }
                (Km::Angle { name, tol: t, safety }, _) => (*name, *t, *safety) = (c.name.clone(), tol, sp),
                _ => {}
            }
            if let (CharacteristicKind::Point { points, .. }, Some(first)) = (&c.kind, inputs.first_mut()) {
                // every point of the group goes into the one input
                first.items = points.iter().map(|f| sel_of(app, f)).collect();
            } else {
                for (i, f) in inputs.iter_mut().zip(c.kind.faces()) {
                    i.items = vec![sel_of(app, f)];
                }
            }
            let mut active = 0;
            sync_points(&km, &mut inputs, &mut active);
            if let (Some(pin), Some(face)) = (inputs.get_mut(1).filter(|i| i.places), c.kind.faces().first()) {
                pin.items = picks
                    .iter()
                    .map(|p| {
                        let at = solidcraft_engine::geom::Vec3::new(p[0], p[1], p[2]);
                        crate::dialogs::face_sel(&app.session, at).unwrap_or(Sel::Face { body: face.body.clone(), index: 0, point: at })
                    })
                    .collect();
            }
            (c.kind.command(), km, inputs)
        }
        _ => return,
    };
    app.start(cmd);
    if let Some(d) = app.dialog.as_mut()
        && matches!(d.kind, Kind::Cmm(_))
    {
        d.kind = Kind::Cmm(km);
        d.inputs = inputs;
        d.extra = extra;
    }
}

fn face_json(s: &Sel) -> Option<Value> {
    match s {
        Sel::Face { body, point, .. } => Some(json!({"body": body, "point": [point.x, point.y, point.z]})),
        _ => None,
    }
}

fn faces_of(inputs: &[SelInput], i: usize) -> Vec<Value> {
    inputs.get(i).map(|x| x.items.iter().filter_map(face_json).collect()).unwrap_or_default()
}

fn face_ref(v: &Value) -> Option<FaceRef> {
    Some(FaceRef { body: v.get("body")?.as_str()?.to_string(), point: serde_json::from_value(v.get("point")?.clone()).ok()? })
}

/// The alignment the dialog's picks make (for the preview of its coordinate system).
pub fn alignment_of(app: &SolveApp, inputs: &[SelInput], clamp: Option<insp::ClampPlane>) -> Alignment {
    let model = app.session.world_state();
    let refs = |i: usize| -> Vec<FaceRef> { faces_of(inputs, i).iter().filter_map(face_ref).collect() };
    let mut a = Alignment {
        primary: refs(0).into_iter().next(),
        direction: match refs(1).as_slice() {
            [f] => Some(DirectionRef::Side { face: f.clone() }),
            [x, y] => Some(DirectionRef::Bores { first: x.clone(), second: y.clone() }),
            _ => None,
        },
        origin_x: None,
        origin_y: None,
        bores: AlignBores::Cylinder,
        clamp,
    };
    // the origins' faces as the part lies (its axes come from the primary and direction faces)
    let axes_of = a.clone();
    let origin = |faces: Vec<FaceRef>| {
        insp::in_alignment(&model, &axes_of, || match faces.as_slice() {
            [f] => Some(match insp::analyze(&model, f).map(|x| x.kind) {
                Ok(insp::FaceKind::Bore { .. }) => OriginRef::Bore { face: f.clone() },
                _ => OriginRef::Side { face: f.clone() },
            }),
            [a, b] if insp::same_bore(&model, a, b) => Some(OriginRef::Bore { face: a.clone() }),
            [a, b] => Some(OriginRef::Middle { first: a.clone(), second: b.clone() }),
            _ => None,
        })
    };
    a.origin_x = origin(refs(2));
    a.origin_y = origin(refs(3));
    if a.origin_y.is_none()
        && let Some(OriginRef::Bore { face }) = &a.origin_x
    {
        a.origin_y = Some(OriginRef::Bore { face: face.clone() });
    }
    a
}

fn number(v: &str) -> Option<f64> {
    v.replace(',', ".").trim().parse::<f64>().ok().filter(|x| x.is_finite())
}

/// The points picked on the face (input 1 of a plane or cylinder dialog).
fn picks_of(inputs: &[SelInput]) -> Vec<[f64; 3]> {
    inputs
        .get(1)
        .filter(|i| i.places)
        .map(|i| {
            i.items
                .iter()
                .filter_map(|s| match s {
                    Sel::Face { point, .. } => Some([point.x, point.y, point.z]),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn count(v: &str, default: u32) -> u32 {
    number(v).filter(|n| *n >= 0.0 && *n <= 1000.0).map_or(default, |n| n.round() as u32)
}

/// The characteristic the dialog describes (for the nominal and the points shown while
/// picking).
pub fn characteristic_of(k: &Km, inputs: &[SelInput]) -> Option<Characteristic> {
    let f = |i: usize| faces_of(inputs, i).first().and_then(face_ref);
    let (name, tol, safety, kind) = match k {
        Km::Circle { name, tol, points, height, position, safety } => (
            name,
            tol.as_str(),
            *safety,
            CharacteristicKind::Circle { face: f(0)?, points: count(points, 4), height: number(height).unwrap_or(2.0), position: *position },
        ),
        Km::Cylinder { name, tol, strategy, paths, points, height, safety } => (
            name,
            tol.as_str(),
            *safety,
            CharacteristicKind::Cylinder {
                face: f(0)?,
                strategy: [CylinderProbing::Scan, CylinderProbing::CirclePoints, CylinderProbing::Points].get(*strategy).copied().unwrap_or_default(),
                paths: count(paths, 2),
                points: count(points, 7),
                height: number(height).unwrap_or(2.0),
                picks: if *strategy == 2 { picks_of(inputs) } else { Vec::new() },
            },
        ),
        Km::Plane { name, strategy, points, safety } => (
            name,
            "0",
            *safety,
            CharacteristicKind::Plane {
                face: f(0)?,
                strategy: if *strategy == 1 { PlaneProbing::Circles } else { PlaneProbing::Points },
                points: count(points, 4),
                picks: if *strategy == 0 { picks_of(inputs) } else { Vec::new() },
            },
        ),
        Km::Distance { name, tol, axis, safety } => {
            let axis = match axis {
                1 => Some(solidcraft_engine::doc::cmm::Axis::X),
                2 => Some(solidcraft_engine::doc::cmm::Axis::Y),
                3 => Some(solidcraft_engine::doc::cmm::Axis::Z),
                _ => None,
            };
            (name, tol.as_str(), *safety, CharacteristicKind::Distance { from: f(0)?, to: f(1)?, axis })
        }
        Km::Angle { name, tol, safety } => (name, tol.as_str(), *safety, CharacteristicKind::Angle { first: f(0)?, second: f(1)? }),
        Km::Point { name, tol, safety, check } => {
            let points: Vec<FaceRef> = faces_of(inputs, 0).iter().filter_map(face_ref).collect();
            if points.is_empty() {
                return None;
            }
            let check = if *check == 1 { PointCheck::Coordinates } else { PointCheck::Normal };
            (name, tol.as_str(), *safety, CharacteristicKind::Point { points, check })
        }
        Km::Setup { .. } | Km::Nc { .. } | Km::Alignment { .. } | Km::ProbeTable { .. } => return None,
    };
    Some(Characteristic {
        id: 0,
        name: name.clone(),
        kind,
        tol: number(tol).unwrap_or(0.05),
        safety_plane: safety_of_index(safety),
        probe_file: None,
    })
}

fn safety_row(ui: &mut egui::Ui, safety: &mut usize) {
    choice_row(ui, "Safety Plane", "cmm_safety", safety, &CHAR_SAFETY);
}

/// The probe configuration of a measurement or characteristic (`extra["probe_file"]`; none:
/// the setup's), from the probe table and the probe files in the data folder.
fn probe_row(ui: &mut egui::Ui, extra: &mut Map<String, Value>) {
    let files: Vec<String> = insp::probe_choices();
    let current = extra.get("probe_file").and_then(Value::as_str).map(str::to_string);
    let mut labels = vec![crate::i18n::tr_label("From the setup").to_string()];
    labels.extend(files.iter().map(|p| path_label(p)));
    // a file chosen earlier that is no longer in the folder stays listed
    let mut files = files;
    if let Some(c) = &current
        && !files.contains(c)
    {
        labels.push(path_label(c));
        files.push(c.clone());
    }
    let mut i = current.as_ref().and_then(|c| files.iter().position(|f| f == c)).map_or(0, |i| i + 1);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let before = i;
    choice_row(ui, "Probe", "cmm_char_probe", &mut i, &refs);
    if i != before {
        let v = i.checked_sub(1).and_then(|i| files.get(i)).map_or(Value::Null, |f| json!(f));
        extra.insert("probe_file".into(), v);
    }
    if let Some(row) = extra.get("probe_file").and_then(Value::as_str).and_then(insp::probe_row) {
        let d = |x: Option<f64>| x.map_or_else(|| "?".to_string(), insp::num);
        ui_kit::text(
            ui,
            &format!("Ø{} · L {} · ML {} · Ø{}", d(row.ball), d(row.length), d(row.measuring_length), d(row.shaft)),
            Text::Note("cmm_char_probe_dims"),
        );
    }
}

fn probe_rows(ui: &mut egui::Ui, rows: &mut Vec<ProbeText>, shown: &mut usize, new_name: &mut String) -> bool {
    let mut enter = false;
    let table = insp::probe_table_path().map(|p| p.display().to_string()).unwrap_or_else(|| "<data>/inspection/probes/probe-table.json".into());
    if rows.is_empty() {
        ui_kit::text(ui, crate::i18n::tr_label("No probes yet: add one below."), Text::Note("cmm_probe_none"));
    } else {
        let names: Vec<String> = rows.iter().map(|r| r.file.clone()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        *shown = (*shown).min(rows.len() - 1);
        choice_row(ui, "Probe", "cmm_probe_row", shown, &refs);
        if let Some(r) = rows.get_mut(*shown) {
            for (v, (label, unit)) in r.dims.iter_mut().zip(PROBE_DIMS) {
                enter |= hint_row(ui, label, v, unit);
            }
            enter |= text_row(ui, "Order No.", &mut r.order_no, "").0;
            enter |= text_row(ui, "Note", &mut r.note, "").0;
        }
        if ui_kit::row(ui, "", |ui| ui_kit::button(ui, crate::i18n::tr_label("Remove this probe"), ui_kit::Button::Normal)).clicked()
            && *shown < rows.len()
        {
            rows.remove(*shown);
            *shown = shown.saturating_sub(1);
        }
    }
    // a new probe: its name as the machine knows it (a probe file of that name is used if present)
    text_row(ui, "New Probe", new_name, crate::i18n::tr_label("name, e.g. as on the machine"));
    let name = new_name.trim().to_string();
    let taken = rows.iter().any(|r| r.file.eq_ignore_ascii_case(&name));
    let ok = !name.is_empty() && !taken && !name.contains(['/', '\\']) && rows.len() < insp::MAX_PROBE_ROWS;
    if ui_kit::row(ui, "", |ui| ui_kit::button(ui, crate::i18n::tr_label("Add probe"), ui_kit::Button::Enabled(ok))).clicked() && ok {
        rows.push(ProbeText { file: name, ..Default::default() });
        *shown = rows.len() - 1;
        new_name.clear();
    }
    ui_kit::text(ui, &format!("{} {table}", crate::i18n::tr_label("Saved for all designs (not in the design):")), Text::Note("cmm_probe_where"));
    enter
}

/// The dialog's own rows (after the selection inputs); true when Enter was pressed in a value.
pub fn rows(
    app: &mut SolveApp,
    ui: &mut egui::Ui,
    k: &mut Km,
    inputs: &mut Vec<SelInput>,
    active: &mut usize,
    extra: &mut Map<String, Value>,
) -> bool {
    let mut enter = false;
    match k {
        Km::Setup { name, probe, probes, safety, plane_probing } => {
            enter |= text_row(ui, "Name", name, "Setup").0;
            let mut p_labels = vec!["From the post".to_string()];
            p_labels.extend(probes.iter().map(|p| path_label(p)));
            let p_refs: Vec<&str> = p_labels.iter().map(String::as_str).collect();
            choice_row(ui, "Probe System", "cmm_probe", probe, &p_refs);
            if let Some(row) = probe.checked_sub(1).and_then(|i| probes.get(i)).and_then(|p| insp::probe_row(p)) {
                let d = |x: Option<f64>| x.map_or_else(|| "?".to_string(), insp::num);
                ui_kit::text(
                    ui,
                    &format!("Ø{} · L {} · ML {} · Ø{}", d(row.ball), d(row.length), d(row.measuring_length), d(row.shaft)),
                    Text::Note("cmm_probe_dims"),
                );
            }
            choice_row(ui, "Safety Plane", "cmm_setup_safety", safety, &SETUP_SAFETY);
            choice_row(ui, "Planes", "cmm_plane_probing", plane_probing, &PLANE_DEFAULTS);
            ui_kit::text(
                ui,
                crate::i18n::tr_label("Postprocessor and plan output: in the setup's NC programs (Create Inspection Plan)."),
                Text::Note("cmm_nc_note"),
            );
            if probes.is_empty() {
                ui_kit::text(ui, crate::i18n::tr_label("No probes yet: add them in the probe table (Probes)."), Text::Note("cmm_pcf"));
            }
        }
        Km::Nc { name, post, posts, plan_name, output_dir, settings, chars, sheet, step } => {
            enter |= text_row(ui, "Name", name, solidcraft_engine::tr!("NC Program")).0;
            let mut t_labels = vec!["Automatic (first post)".to_string()];
            t_labels.extend(posts.iter().cloned());
            let t_refs: Vec<&str> = t_labels.iter().map(String::as_str).collect();
            choice_row(ui, "Postprocessor", "cmm_nc_post", post, &t_refs);
            if posts.is_empty() {
                let folder = insp::data_folder().map(|d| d.display().to_string()).unwrap_or_else(|| "<data>/inspection".into());
                ui_kit::text(ui, &format!("No postprocessor yet: put the post's program into {folder}/post."), Text::Note("cmm_tpl"));
            }
            enter |= text_row(ui, "Plan Name", plan_name, crate::i18n::tr_label("design and setup name")).0;
            enter |= text_row(ui, "Output Folder", output_dir, crate::i18n::tr_label("<data>/inspection/output")).0;
            if app.services.pick_folder.is_some()
                && ui_kit::row(ui, "", |ui| ui_kit::button(ui, crate::i18n::tr_label("Choose folder…"), ui_kit::Button::Normal)).clicked()
            {
                let title = crate::i18n::tr_label("Output Folder").to_string();
                app.pick_folder(&title, |app, path| {
                    if let Some(path) = path
                        && let Some(d) = app.dialog.as_mut()
                        && let Kind::Cmm(Km::Nc { output_dir, .. }) = &mut d.kind
                    {
                        *output_dir = path;
                    }
                });
            }
            ui_kit::checkbox(ui, crate::i18n::tr_label("Setup sheet (PDF) for the operator"), sheet);
            ui_kit::checkbox(ui, crate::i18n::tr_label("CAD model (STEP) in the base system"), step);
            if let Some(list) = settings.get_mut(*post) {
                if !list.is_empty() {
                    ui_kit::text(ui, crate::i18n::tr_label("Postprocessor settings"), Text::Note("cmm_nc_settings"));
                }
                for s in list.iter_mut() {
                    if s.flag {
                        ui_kit::checkbox(ui, &s.label, &mut s.on);
                    } else {
                        enter |= text_row(ui, &s.label, &mut s.text, "").0;
                    }
                }
            }
            ui_kit::text(ui, crate::i18n::tr_label("Characteristics in this program"), Text::Note("cmm_nc_chars"));
            if chars.is_empty() {
                ui_kit::text(ui, crate::i18n::tr_label("No characteristics yet."), Text::Note("cmm_nc_none"));
            }
            for (_, label, on) in chars.iter_mut() {
                ui_kit::checkbox(ui, label, on);
            }
        }
        Km::ProbeTable { rows, shown, new_name } => enter |= probe_rows(ui, rows, shown, new_name),
        Km::Alignment { bores, clamp, margin, points } => {
            ui_kit::text(
                ui,
                crate::i18n::tr_label(
                    "Primary plane: a plane of the part (spatial axis, origin Z); the measuring axes follow the part, however it lies.",
                ),
                Text::Note("cmm_a1"),
            );
            ui_kit::checkbox(ui, crate::i18n::tr_label("Clamped on this face: probe the clamping plate beside the part"), clamp);
            if *clamp {
                enter |= hint_row(ui, "Distance from the part", margin, "10 mm");
                enter |= text_row(ui, "Points on the plate", points, crate::i18n::tr_label("X Y; X Y; … (empty: 4 points around the part)")).0;
            }
            ui_kit::text(
                ui,
                crate::i18n::tr_label("Direction: one side face, or two bores (X runs from the first to the second)."),
                Text::Note("cmm_a2"),
            );
            ui_kit::text(
                ui,
                crate::i18n::tr_label("Origin X / Y: a side face, two opposite sides (their middle) or a bore (its axis; for X alone it is Y too)."),
                Text::Note("cmm_a3"),
            );
            ui_kit::text(
                ui,
                crate::i18n::tr_label("How a face is probed: a plane or cylinder measurement of it, else the setup's planes setting."),
                Text::Note("cmm_a4"),
            );
            choice_row(ui, "Bores", "cmm_align_bores", bores, &ALIGN_BORES);
            let note = if *bores == 1 {
                "Circle: probed below the opening (a circle measurement of the bore gives points and height, else 4 points 2 mm below); its centre is the origin."
            } else {
                "Cylinder: scanned over the depth; where its axis meets the primary plane is the origin."
            };
            ui_kit::text(ui, crate::i18n::tr_label(note), Text::Note("cmm_a5"));
            let a =
                alignment_of(app, inputs, clamp_of(&Km::Alignment { bores: *bores, clamp: *clamp, margin: margin.clone(), points: points.clone() }));
            if a.is_complete() {
                match insp::frame(&app.session.world_state(), &a) {
                    Ok(f) => {
                        let o = f.origin.map(insp::num);
                        ui_kit::text(ui, &format!("Origin X{} Y{} Z{}", o[0], o[1], o[2]), Text::Value("cmm_origin", false));
                    }
                    Err(e) => {
                        ui_kit::text(ui, &e, Text::Value("cmm_origin", true));
                    }
                }
            }
        }
        Km::Circle { name, tol, points, height, position, safety } => {
            enter |= hint_row(ui, "Tolerance ±", tol, "mm");
            enter |= hint_row(ui, "Points", points, "4");
            enter |= hint_row(ui, "Distance to the Plane", height, "2 mm");
            ui_kit::checkbox(ui, crate::i18n::tr_label("Position from the origin (X, Y)"), position);
            safety_row(ui, safety);
            probe_row(ui, extra);
            enter |= text_row(ui, "Name", name, crate::i18n::tr_label("automatic")).0;
        }
        Km::Cylinder { name, tol, strategy, paths, points, height, safety } => {
            choice_row(ui, "Strategy", "cmm_cyl_strategy", strategy, &CYLINDER_STRATEGIES);
            enter |= hint_row(ui, "Tolerance ±", tol, "mm");
            if *strategy < 2 {
                enter |= hint_row(ui, "Paths", paths, "2");
                if *strategy == 1 {
                    enter |= hint_row(ui, "Points per Path", points, "7");
                }
                enter |= hint_row(ui, "Distance to the Planes", height, "2 mm");
            } else {
                ui_kit::text(ui, crate::i18n::tr_label("Click at least 4 points on the bore's wall."), Text::Note("cmm_cyl_pick"));
            }
            safety_row(ui, safety);
            probe_row(ui, extra);
            enter |= text_row(ui, "Name", name, crate::i18n::tr_label("automatic")).0;
        }
        Km::Plane { name, strategy, points, safety } => {
            choice_row(ui, "Strategy", "cmm_plane_strategy", strategy, &PLANE_STRATEGIES);
            if *strategy == 0 {
                enter |= hint_row(ui, "Points", points, "4");
                ui_kit::text(
                    ui,
                    crate::i18n::tr_label("Click at least 4 points on the face, or none to spread the points over it."),
                    Text::Note("cmm_plane_pick"),
                );
            }
            ui_kit::text(ui, crate::i18n::tr_label("Holds wherever the plane is used (alignment, distances)."), Text::Note("cmm_plane_where"));
            safety_row(ui, safety);
            probe_row(ui, extra);
            enter |= text_row(ui, "Name", name, crate::i18n::tr_label("automatic")).0;
        }
        Km::Point { name, tol, safety, check } => {
            let n = inputs.first().map_or(0, |i| i.items.len());
            ui_kit::text(
                ui,
                &format!("{} {n}", crate::i18n::tr_label("Click one or many points on the part, e.g. along a contour. Points:")),
                Text::Note("cmm_points_note"),
            );
            choice_row(ui, "Evaluation", "cmm_point_check", check, &["Deviation along the normal (form)", "Coordinates X, Y, Z"]);
            enter |= hint_row(ui, "Tolerance ±", tol, "mm");
            safety_row(ui, safety);
            probe_row(ui, extra);
            enter |= text_row(ui, "Name", name, crate::i18n::tr_label("automatic")).0;
        }
        Km::Distance { name, tol, axis, safety } => {
            choice_row(ui, "Axis", "cmm_axis", axis, &AXES);
            enter |= hint_row(ui, "Tolerance ±", tol, "mm");
            safety_row(ui, safety);
            probe_row(ui, extra);
            enter |= text_row(ui, "Name", name, crate::i18n::tr_label("automatic")).0;
        }
        Km::Angle { name, tol, safety } => {
            enter |= hint_row(ui, "Tolerance ±", tol, "deg");
            safety_row(ui, safety);
            probe_row(ui, extra);
            enter |= text_row(ui, "Name", name, crate::i18n::tr_label("automatic")).0;
        }
    }
    sync_points(k, inputs, active);
    if let Some(c) = characteristic_of(k, inputs) {
        let model = app.session.world_state();
        let setup = app.session.doc.cmm.active_setup();
        let frame = setup.and_then(|s| insp::frame(&model, &s.alignment).ok());
        let al = setup.map(|s| s.alignment.clone()).unwrap_or_default();
        let info = insp::in_alignment(&model, &al, || insp::characteristic_info(&model, frame.as_ref(), &c));
        let problem = info["error"].as_str().map(str::to_string).or_else(|| setup.and_then(|s| insp::characteristic_elements(&model, s, &c).err()));
        match (info["nominal"].as_str(), problem) {
            (_, Some(e)) => {
                ui_kit::text(ui, &e, Text::Value("cmm_nominal", true));
            }
            (Some(n), None) => {
                let label = if matches!(k, Km::Plane { .. }) { n.to_string() } else { format!("Nominal {n}") };
                ui_kit::text(ui, &label, Text::Value("cmm_nominal", false));
            }
            _ => {}
        }
    }
    enter
}

/// The commands the dialog's OK runs.
pub fn commands(app: &SolveApp, k: &Km, inputs: &[SelInput], extra: &Map<String, Value>) -> Result<Vec<(String, Value)>, String> {
    let one = |i: usize, what: &str| faces_of(inputs, i).into_iter().next().ok_or_else(|| format!("select {what} first"));
    let tol = |t: &str| number(t).filter(|x| *x >= 0.0).ok_or_else(|| "Tolerance: enter a number".to_string());
    let mm = |t: &str, what: &str| number(t).filter(|x| *x >= 0.0).ok_or_else(|| format!("{what}: enter mm"));
    let n_in = |t: &str, lo: f64, hi: f64, what: &str| {
        number(t).filter(|n| (lo..=hi).contains(n) && n.fract() == 0.0).map(|n| n as u32).ok_or_else(|| format!("{what}: {lo} to {hi}"))
    };
    let picks = || -> Result<Vec<[f64; 3]>, String> {
        let p = picks_of(inputs);
        if p.len() < insp::MIN_PICKS {
            Err(format!("Points: click at least {} points on the face ({} so far)", insp::MIN_PICKS, p.len()))
        } else {
            Ok(p)
        }
    };
    let mut p = match k {
        Km::Setup { name, probe, probes, safety, plane_probing } => {
            let pick = |list: &[String], i: usize| i.checked_sub(1).and_then(|i| list.get(i)).map_or(Value::Null, |p| json!(p));
            json!({
                "name": name, "probe_file": pick(probes, *probe),
                "safety_plane": safety_json(safety + 1),
                "plane_probing": if *plane_probing == 1 { "circles" } else { "points" },
            })
        }
        Km::Nc { name, post, posts, plan_name, output_dir, settings, chars, sheet, step } => {
            let options: Map<String, Value> = settings
                .get(*post)
                .into_iter()
                .flatten()
                .map(|s| (s.key.clone(), json!(if s.flag { if s.on { "on".to_string() } else { "off".to_string() } } else { s.text.clone() })))
                .collect();
            let chosen: Vec<u64> = chars.iter().filter(|c| c.2).map(|c| c.0).collect();
            let all = chosen.len() == chars.len();
            let dir = output_dir.trim();
            json!({
                "name": name,
                "post": post.checked_sub(1).and_then(|i| posts.get(i)).map_or(Value::Null, |p| json!(p)),
                "plan_name": plan_name,
                "output_dir": if dir.is_empty() { Value::Null } else { json!(dir) },
                "options": options,
                "characteristics": if all { Value::Null } else { json!(chosen) },
                "sheet": sheet,
                "step": step,
                "export": true,
            })
        }
        Km::ProbeTable { rows, .. } => {
            let mut out = Vec::new();
            for r in rows {
                let mut d = [None; 7];
                for (slot, (v, (label, _))) in d.iter_mut().zip(r.dims.iter().zip(PROBE_DIMS)) {
                    if !v.trim().is_empty() {
                        *slot = Some(number(v).filter(|x| *x > 0.0).ok_or_else(|| format!("{}: {label}: enter mm", r.file))?);
                    }
                }
                out.push(json!({
                    "file": r.file, "ball": d[0], "length": d[1], "measuring_length": d[2], "shaft": d[3],
                    "measuring_length2": d[4], "shaft2": d[5], "body": d[6], "order_no": r.order_no, "note": r.note,
                }));
            }
            json!({ "rows": out })
        }
        Km::Alignment { bores, clamp, margin, points } => {
            let need = |i: usize, what: &str| {
                let f = faces_of(inputs, i);
                if f.is_empty() || f.len() > 2 { Err(format!("{what}: select one or two faces")) } else { Ok(f) }
            };
            let primary = one(0, "the primary plane")?;
            let mut p = json!({"primary": primary, "direction": need(1, "Direction")?, "origin_x": need(2, "Origin X")?});
            let y = faces_of(inputs, 3);
            if y.len() > 2 {
                return Err("Origin Y: select one or two faces".into());
            }
            p["origin_y"] = json!(y);
            p["bores"] = json!(if *bores == 1 { "circle" } else { "cylinder" });
            p["clamp"] = json!(clamp);
            if *clamp {
                if !margin.trim().is_empty() {
                    p["clamp_margin"] = json!(mm(margin, "Distance from the part")?);
                }
                let pts = parse_points(points).ok_or_else(|| "Points on the plate: X Y; X Y; … in mm".to_string())?;
                p["clamp_points"] = json!(pts);
            }
            if let Some(s) = app.session.doc.cmm.active_setup() {
                p["setup"] = json!(s.id);
            }
            p
        }
        Km::Circle { name, tol: t, points, height, position, safety } => json!({
            "face": one(0, "a bore")?, "tol": tol(t)?, "points": n_in(points, 3.0, 100.0, "Points")?,
            "height": mm(height, "Distance to the plane")?, "position": position, "name": name, "safety_plane": safety_json(*safety),
        }),
        Km::Cylinder { name, tol: t, strategy, paths, points, height, safety } => {
            let mut p = json!({"face": one(0, "a bore")?, "tol": tol(t)?, "name": name, "safety_plane": safety_json(*safety)});
            match strategy {
                2 => {
                    p["strategy"] = json!("points");
                    p["picks"] = json!(picks()?);
                }
                s => {
                    p["strategy"] = json!(if *s == 1 { "circle_points" } else { "scan" });
                    p["paths"] = json!(n_in(paths, 1.0, 20.0, "Paths")?);
                    p["height"] = json!(mm(height, "Distance to the planes")?);
                    if *s == 1 {
                        p["points"] = json!(n_in(points, 3.0, 100.0, "Points per path")?);
                    }
                }
            }
            p
        }
        Km::Plane { name, strategy, points, safety } => {
            let mut p = json!({"face": one(0, "a plane")?, "name": name, "safety_plane": safety_json(*safety)});
            if *strategy == 1 {
                p["strategy"] = json!("circles");
            } else {
                p["strategy"] = json!("points");
                if picks_of(inputs).is_empty() {
                    p["points"] = json!(n_in(points, 3.0, 100.0, "Points")?);
                } else {
                    p["picks"] = json!(picks()?);
                }
            }
            p
        }
        Km::Distance { name, tol: t, axis, safety } => {
            let mut p = json!({"from": one(0, "the first element")?, "to": one(1, "the second element")?, "tol": tol(t)?, "name": name, "safety_plane": safety_json(*safety)});
            if let Some(a) = ["x", "y", "z"].get(axis.wrapping_sub(1)) {
                p["axis"] = json!(a);
            }
            p
        }
        Km::Angle { name, tol: t, safety } => {
            json!({"first": one(0, "the first plane")?, "second": one(1, "the second plane")?, "tol": tol(t)?, "name": name, "safety_plane": safety_json(*safety)})
        }
        Km::Point { name, tol: t, safety, check } => {
            let points = faces_of(inputs, 0);
            if points.is_empty() {
                return Err("click at least one point on the part".into());
            }
            json!({
                "points": points, "check": if *check == 1 { "coordinates" } else { "normal" },
                "tol": tol(t)?, "name": name, "safety_plane": safety_json(*safety),
            })
        }
    };
    if let Value::Object(m) = &mut p {
        for (key, v) in extra {
            m.insert(key.clone(), v.clone());
        }
    }
    Ok(vec![(k.command().to_string(), p)])
}

#[cfg(test)]
#[path = "dialogs_cmm_tests.rs"]
pub(crate) mod tests;
