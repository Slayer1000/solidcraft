//! Coordinate measuring (CMM) plans on the model. Each setup becomes a neutral CMM
//! program ([`solidcraft_inspection::program`]) that a postprocessor translates into one
//! measuring machine's plan, as a CAM post does for NC code.
//!
//! The plans are saved with the design (`Document::cmm`): setups, each with a probe system, an
//! alignment (3-2-1: primary plane, direction, origin X/Y) and characteristics. Faces are kept
//! as a body and a point on the face, so they follow rebuilds; everything here reads their
//! geometry again from the evaluated model.
//!
//! Supported faces: planes parallel to two axes (probed along circular paths placed
//! automatically on the face) and vertical bores (scanned along the axis); an angle also takes
//! inclined planes. Postprocessors and probe files live only in the user's data folder
//! (`<data>/inspection/post`, `<data>/inspection/probes`), never in the repository.

use crate::tr;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use solidcraft_geom::Vec3;
use solidcraft_inspection::program as pg;

pub use solidcraft_doc::cmm::{
    AlignBores, Alignment, Axis, Characteristic, CharacteristicKind, ClampPlane, CmmPlans, CylinderProbing, DirectionRef, FaceRef,
    MAX_CHARACTERISTICS, MAX_CLAMP_POINTS, MAX_NC_PROGRAMS, MAX_PICKS, MAX_POINTS, MAX_SETUPS, NcProgram, OriginRef, PlaneProbing, PointCheck,
    ProbeSystem, SafetyPlane, Setup,
};

// ------------------------------------------------------------------------------------------
// The part's orientation
// ------------------------------------------------------------------------------------------
//
// A part may lie turned in the model. The measuring coordinate system follows the part, not the
// model: the primary face gives Z (out of the face, or into the part from the face it is clamped
// on), the direction face turns X and Y. Inside [`in_alignment`] every face is analysed in these
// part axes (so a turned part has axis-parallel planes and upright bores like any other) and
// everything worked out there is in part axes; the public results ([`frame`], [`program`],
// [`characteristic_elements`], the markers) are turned back into model coordinates.

/// The part's axes in model coordinates (unit, right-handed). Part coordinates are the model's
/// turned onto these axes (no shift).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartAxes {
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub z: [f64; 3],
}

impl PartAxes {
    fn to_part(self, v: [f64; 3]) -> [f64; 3] {
        [dot3(v, self.x), dot3(v, self.y), dot3(v, self.z)]
    }
    fn to_model(self, v: [f64; 3]) -> [f64; 3] {
        [0, 1, 2].map(|i| self.x[i] * v[0] + self.y[i] * v[1] + self.z[i] * v[2])
    }
}

thread_local! {
    /// Inside [`in_alignment`]: the part's axes (`Some(None)`: the model's own).
    static PART: std::cell::Cell<Option<Option<PartAxes>>> = const { std::cell::Cell::new(None) };
}

fn part_axes_now() -> Option<PartAxes> {
    PART.with(|p| p.get().flatten())
}

/// A model point or direction in part axes (unchanged outside [`in_alignment`]).
pub fn to_part(v: [f64; 3]) -> [f64; 3] {
    part_axes_now().map_or(v, |a| a.to_part(v))
}

/// A point or direction in part axes back in model coordinates.
pub fn to_model(v: [f64; 3]) -> [f64; 3] {
    part_axes_now().map_or(v, |a| a.to_model(v))
}

fn unit3(v: [f64; 3]) -> Option<[f64; 3]> {
    let l = dot3(v, v).sqrt();
    (l.is_finite() && l > 1e-9).then(|| v.map(|c| c / l))
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The part's axes an alignment gives, `None` when they are the model's: Z along the primary
/// face's outward normal (against it for a clamping face), X and Y turned about Z so that the
/// direction face stands across one of them, starting from the model's X.
pub fn part_axes(model: &solidcraft_doc::ModelState, a: &Alignment) -> Option<PartAxes> {
    let n = plane_normal(model, a.primary.as_ref()?).ok()?;
    let z = unit3(if a.clamp.is_some() { n.map(|c| -c) } else { n })?;
    let pick = if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let d = dot3(pick, z);
    let mut x = unit3([0, 1, 2].map(|i| pick[i] - z[i] * d))?;
    if let Some(DirectionRef::Side { face }) = &a.direction
        && let Ok(n) = plane_normal(model, face)
    {
        let y = cross3(z, x);
        let (u, w) = (dot3(n, x), dot3(n, y));
        if u.hypot(w) > 0.1 {
            let quarter = std::f64::consts::FRAC_PI_2;
            let angle = w.atan2(u);
            let turn = angle - (angle / quarter).round() * quarter;
            let (c, s) = (turn.cos(), turn.sin());
            x = unit3([0, 1, 2].map(|i| x[i] * c + y[i] * s))?;
        }
    }
    let y = cross3(z, x);
    let same = |v: [f64; 3], e: [f64; 3]| (0..3).all(|i| (v[i] - e[i]).abs() < 1e-9);
    if same(x, [1.0, 0.0, 0.0]) && same(y, [0.0, 1.0, 0.0]) && same(z, [0.0, 0.0, 1.0]) {
        return None;
    }
    Some(PartAxes { x, y, z })
}

/// Resets the part axes when the scope ends (also when it unwinds).
struct PartScope;

impl Drop for PartScope {
    fn drop(&mut self) {
        PART.with(|p| p.set(None));
    }
}

/// Runs `f` with faces analysed in the part axes of alignment `a` (see the section's note).
/// Nested calls keep the outer scope.
pub fn in_alignment<T>(model: &solidcraft_doc::ModelState, a: &Alignment, f: impl FnOnce() -> T) -> T {
    if PART.with(|p| p.get().is_some()) {
        return f();
    }
    let axes = part_axes(model, a);
    PART.with(|p| p.set(Some(axes)));
    let _scope = PartScope;
    f()
}

/// An element worked out in part axes, in model coordinates.
fn element_to_model(e: &mut pg::Element) {
    let Some(ax) = part_axes_now() else { return };
    let m = |v: &mut [f64; 3]| *v = ax.to_model(*v).map(round);
    match &mut e.geometry {
        pg::Geometry::Plane { normal, point, min, max } => {
            m(normal);
            m(point);
            m(min);
            m(max);
        }
        pg::Geometry::Cylinder { top, axis, .. } => {
            m(top);
            m(axis);
        }
        pg::Geometry::Circle { center, normal, .. } => {
            m(center);
            m(normal);
        }
        pg::Geometry::Point { point, normal } => {
            m(point);
            m(normal);
        }
    }
    match &mut e.strategy {
        pg::Strategy::CirclesOnPlane { circles } => circles.iter_mut().for_each(|c| m(&mut c.center)),
        pg::Strategy::Points { at, .. } => at.iter_mut().for_each(m),
        pg::Strategy::CylinderScan { .. } | pg::Strategy::CylinderPoints { .. } => {}
    }
}

/// Markers worked out in part axes ([`element_markers`]), in model coordinates.
fn markers_to_model(markers: &mut [Value]) {
    let Some(ax) = part_axes_now() else { return };
    for mk in markers.iter_mut() {
        for key in ["at", "normal", "center", "start_normal"] {
            let v = mk.get(key).and_then(|v| Some([v.get(0)?.as_f64()?, v.get(1)?.as_f64()?, v.get(2)?.as_f64()?]));
            if let Some(v) = v {
                mk[key] = json!(ax.to_model(v).map(round));
            }
        }
    }
}

/// Outward normal of an axis-parallel plane face: the six axis directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Normal {
    PlusX,
    MinusX,
    PlusY,
    MinusY,
    PlusZ,
    MinusZ,
}

impl Normal {
    pub fn vector(self) -> [f64; 3] {
        match self {
            Normal::PlusX => [1.0, 0.0, 0.0],
            Normal::MinusX => [-1.0, 0.0, 0.0],
            Normal::PlusY => [0.0, 1.0, 0.0],
            Normal::MinusY => [0.0, -1.0, 0.0],
            Normal::PlusZ => [0.0, 0.0, 1.0],
            Normal::MinusZ => [0.0, 0.0, -1.0],
        }
    }
    /// 1 = X, 2 = Y, 3 = Z.
    pub fn axis(self) -> u8 {
        match self {
            Normal::PlusX | Normal::MinusX => 1,
            Normal::PlusY | Normal::MinusY => 2,
            Normal::PlusZ | Normal::MinusZ => 3,
        }
    }
    /// `+X`, `-Y`, …
    pub fn label(self) -> &'static str {
        match self {
            Normal::PlusX => "+X",
            Normal::MinusX => "-X",
            Normal::PlusY => "+Y",
            Normal::MinusY => "-Y",
            Normal::PlusZ => "+Z",
            Normal::MinusZ => "-Z",
        }
    }
    /// The normal nearest to `v`, when `v` is within 1° of an axis.
    pub fn from_vector(v: [f64; 3]) -> Option<Normal> {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if !(l > 0.0) {
            return None;
        }
        let u = [v[0] / l, v[1] / l, v[2] / l];
        let cos1 = 1f64.to_radians().cos();
        [Normal::PlusX, Normal::MinusX, Normal::PlusY, Normal::MinusY, Normal::PlusZ, Normal::MinusZ].into_iter().find(|n| {
            let a = n.vector();
            a[0] * u[0] + a[1] * u[1] + a[2] * u[2] >= cos1
        })
    }
}

// ------------------------------------------------------------------------------------------
// Folders
// ------------------------------------------------------------------------------------------

/// `<data>/inspection`.
pub fn data_folder() -> Option<PathBuf> {
    crate::recovery::data_dir().map(|d| d.join("inspection"))
}

/// A folder in `<data>/inspection`: `name`, or `earlier` (the name the first versions used)
/// while only that one exists.
fn sub_folder(name: &str, earlier: &str) -> Option<PathBuf> {
    let d = data_folder()?;
    let (new, old) = (d.join(name), d.join(earlier));
    Some(if !new.exists() && old.is_dir() { old } else { new })
}

/// `<data>/inspection/probes`: the probe configuration files and the probe table.
pub fn probe_folder() -> Option<PathBuf> {
    sub_folder("probes", "taster")
}

#[cfg(windows)]
fn runnable(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["exe", "cmd", "bat"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

#[cfg(unix)]
fn runnable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(any(windows, unix)))]
fn runnable(_: &Path) -> bool {
    false
}

/// The postprocessors in `<data>/inspection/post`: the programs there, sorted by name.
pub fn posts() -> Vec<PathBuf> {
    let Some(dir) = data_folder().map(|d| d.join("post")) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_file() && runnable(p)).take(1000).collect();
    v.sort();
    v
}

/// A post's name: its file name without the extension.
pub fn post_name(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string()
}

/// The post an NC program uses: its own (a name in the post folder, or a path), else the first
/// one.
pub fn post_path(nc: &NcProgram) -> Option<PathBuf> {
    match nc.post.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(given) if given.contains(['/', '\\']) => Some(PathBuf::from(given)),
        Some(given) => posts().into_iter().find(|x| post_name(x) == given || x.file_name().and_then(|f| f.to_str()) == Some(given)),
        None => posts().into_iter().next(),
    }
}

/// Runs a postprocessor with `args`; its last JSON object on stdout is the answer.
pub fn run_post_with(post: &std::path::Path, args: &[std::ffi::OsString], timeout: std::time::Duration) -> Result<Value, String> {
    use std::io::Read as _;
    let mut command = std::process::Command::new(post);
    command.args(args);
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // no console window flashing up
        command.creation_flags(0x0800_0000);
    }
    let label = post.display().to_string();
    let mut child = command.spawn().map_err(|e| tr!("Postprocessor {label} does not start: {e}", label = label, e = e))?;
    let pipe = |r: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.by_ref().take(16 << 20).read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = pipe(child.stdout.take().map(|x| Box::new(x) as Box<dyn std::io::Read + Send>));
    let err = pipe(child.stderr.take().map(|x| Box::new(x) as Box<dyn std::io::Read + Send>));
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(tr!("Postprocessor {label} did not answer within {s} s", label = label, s = timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(e) => return Err(tr!("Postprocessor {label}: {e}", label = label, e = e)),
        }
    };
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    let (out, err) = (String::from_utf8_lossy(&out), String::from_utf8_lossy(&err));
    if !status.success() {
        let why = err.trim();
        return Err(tr!(
            "Postprocessor {label}: {e}",
            label = post_name(post),
            e = if why.is_empty() { tr!("stopped without a message") } else { why }
        ));
    }
    out.lines()
        .rev()
        .find_map(|l| serde_json::from_str::<Value>(l.trim()).ok().filter(Value::is_object))
        .ok_or_else(|| tr!("Postprocessor {label}: no answer (JSON) on stdout", label = post_name(post)))
}

/// The settings a postprocessor offers: what it answers to `--describe`
/// (`{options: [{key, label, kind: bool|text, default}]}`); none when it cannot describe itself.
pub fn post_settings(post: &Path) -> Vec<Value> {
    let answer = run_post_with(post, &["--describe".into()], std::time::Duration::from_secs(15)).ok();
    let mut options = Vec::new();
    for o in answer.as_ref().and_then(|a| a.get("options")).and_then(Value::as_array).into_iter().flatten().take(100) {
        let (Some(key), Some(label)) = (o.get("key").and_then(Value::as_str), o.get("label").and_then(Value::as_str)) else { continue };
        if key.is_empty() || key.len() > 100 || label.len() > 200 {
            continue;
        }
        let kind = if o.get("kind").and_then(Value::as_str) == Some("bool") { "bool" } else { "text" };
        let default: String = o.get("default").and_then(Value::as_str).unwrap_or_default().chars().take(4096).collect();
        options.push(json!({"key": key, "label": label, "kind": kind, "default": default}));
    }
    options
}

/// The probe configuration files in [`probe_folder`]: every file there but the probe table
/// (whatever format the measuring software's postprocessor reads).
pub fn probe_files() -> Vec<PathBuf> {
    let Some(dir) = probe_folder() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or(".");
            p.is_file() && !name.starts_with('.') && !name.to_ascii_lowercase().ends_with(".json")
        })
        .take(1000)
        .collect();
    v.sort();
    v
}

/// What an NC program outputs: the setup with only the characteristics the program outputs.
/// The measurements the alignment takes its strategy from always stay (the alignment must).
pub fn for_program(model: &solidcraft_doc::ModelState, setup: &Setup, nc: &NcProgram) -> Setup {
    let mut out = setup.clone();
    let keep: Vec<u64> = alignment_elements(model, setup).as_array().into_iter().flatten().filter_map(|e| e["characteristic"].as_u64()).collect();
    out.characteristics.retain(|c| nc.outputs(c.id) || keep.contains(&c.id));
    out
}

/// The NC program a setup outputs through when none is named: its first, else the defaults.
pub fn default_program(setup: &Setup) -> NcProgram {
    setup.programs.first().cloned().unwrap_or_else(|| NcProgram::new(0, setup.next_program_name(tr!("NC Program"))))
}

/// An NC program, described (for the tree and its dialog).
pub fn program_info(setup: &Setup, nc: &NcProgram, design_path: Option<&str>) -> Value {
    let total = setup.characteristics.len();
    let chosen = setup.characteristics.iter().filter(|c| nc.outputs(c.id)).count();
    json!({
        "id": nc.id,
        "name": nc.name,
        "post": nc.post,
        "post_used": post_path(nc).map(|p| post_name(&p)),
        "plan_name": nc.plan_name,
        "plan": plan_name(setup, nc, design_path),
        "output_dir": nc.output_dir,
        "output_used": output_dir(nc).map(|p| p.display().to_string()),
        "options": nc.options,
        "characteristics": nc.characteristics,
        "outputs": format!("{chosen}/{total}"),
    })
}

/// Where an NC program's plans are written.
pub fn output_dir(nc: &NcProgram) -> Option<PathBuf> {
    match &nc.output_dir {
        Some(d) => Some(PathBuf::from(d)),
        None => sub_folder("output", "ausgabe"),
    }
}

/// Plan name: the NC program's own, else the design file's name and the setup's (and, after
/// the setup's first NC program, the program's: two programs never write the same plan).
pub fn plan_name(setup: &Setup, nc: &NcProgram, design_path: Option<&str>) -> String {
    let given = nc.plan_name.trim();
    let raw = if !given.is_empty() {
        given.to_string()
    } else {
        let design = design_path.and_then(|p| Path::new(p).file_stem()).and_then(|s| s.to_str()).unwrap_or("SC_Pruefplan");
        let first = setup.programs.first().is_none_or(|p| p.id == nc.id);
        if first { format!("{design}_{}", setup.name) } else { format!("{design}_{}_{}", setup.name, nc.name) }
    };
    raw.chars().map(|c| if matches!(c, '\'' | '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ') { '_' } else { c }).take(120).collect()
}

// ------------------------------------------------------------------------------------------
// Face analysis
// ------------------------------------------------------------------------------------------

/// What a face is, for inspection.
#[derive(Clone, Debug, PartialEq)]
pub enum FaceKind {
    /// A vertical bore: axis position, diameter, top height and depth.
    Bore { x: f64, y: f64, diameter: f64, top: f64, depth: f64 },
    /// A plane parallel to two axes, with its bounding box.
    Plane { normal: Normal, min: [f64; 3], max: [f64; 3] },
}

/// A resolved face.
#[derive(Clone, Debug)]
pub struct FaceInfo {
    pub body: String,
    /// The first face of its surface patch (both halves of a cylinder are one patch).
    pub index: usize,
    pub kind: FaceKind,
    tris: Vec<[Vec3; 3]>,
}

impl FaceInfo {
    /// Same face (same body and patch).
    pub fn same(&self, o: &FaceInfo) -> bool {
        self.body == o.body && self.index == o.index
    }
}

fn v3(p: [f64; 3]) -> Vec3 {
    Vec3 { x: p[0], y: p[1], z: p[2] }
}

fn arr(v: Vec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 { x: a.x - b.x, y: a.y - b.y, z: a.z - b.z }
}

fn dotv(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn crossv(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 { x: a.y * b.z - a.z * b.y, y: a.z * b.x - a.x * b.z, z: a.x * b.y - a.y * b.x }
}

/// Squared distance from `p` to triangle `t`.
fn dist2_triangle(p: Vec3, t: &[Vec3; 3]) -> f64 {
    let [a, b, c] = *t;
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dotv(ab, ap), dotv(ac, ap));
    let q = if d1 <= 0.0 && d2 <= 0.0 {
        a
    } else {
        let bp = sub(p, b);
        let (d3, d4) = (dotv(ab, bp), dotv(ac, bp));
        if d3 >= 0.0 && d4 <= d3 {
            b
        } else {
            let cp = sub(p, c);
            let (d5, d6) = (dotv(ab, cp), dotv(ac, cp));
            if d6 >= 0.0 && d5 <= d6 {
                c
            } else {
                let vc = d1 * d4 - d3 * d2;
                let vb = d5 * d2 - d1 * d6;
                let va = d3 * d6 - d5 * d4;
                if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
                    let v = d1 / (d1 - d3);
                    Vec3 { x: a.x + ab.x * v, y: a.y + ab.y * v, z: a.z + ab.z * v }
                } else if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                    let w = d2 / (d2 - d6);
                    Vec3 { x: a.x + ac.x * w, y: a.y + ac.y * w, z: a.z + ac.z * w }
                } else if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
                    let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
                    let bc = sub(c, b);
                    Vec3 { x: b.x + bc.x * w, y: b.y + bc.y * w, z: b.z + bc.z * w }
                } else {
                    let den = va + vb + vc;
                    if den.abs() < 1e-300 {
                        a
                    } else {
                        let (v, w) = (vb / den, vc / den);
                        Vec3 { x: a.x + ab.x * v + ac.x * w, y: a.y + ab.y * v + ac.y * w, z: a.z + ab.z * v + ac.z * w }
                    }
                }
            }
        }
    };
    let d = sub(p, q);
    dotv(d, d)
}

/// Resolves a face reference on the evaluated model.
pub fn analyze(state: &solidcraft_doc::ModelState, r: &FaceRef) -> Result<FaceInfo, String> {
    let b = state.body(&r.body).ok_or_else(|| tr!("Body {body} does not exist (any more)", body = format!("{:?}", r.body)))?;
    let m = b.mesh();
    let p = v3(r.point);
    if !r.point.iter().all(|c| c.is_finite()) {
        return Err(tr!("invalid point").into());
    }
    let nearest = m
        .triangles
        .iter()
        .zip(&m.tri_face)
        .filter_map(|(t, f)| m.tri(t).map(|tri| (dist2_triangle(p, &tri), *f as usize)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .ok_or_else(|| tr!("Body {body} has no faces", body = format!("{:?}", r.body)))?;
    let tol = (m.bounds().diagonal().max(1.0) * 1e-3).powi(2);
    if nearest.0 > tol {
        return Err(tr!("there is no face of {body} at {point}", body = format!("{:?}", r.body), point = format!("{:?}", r.point)));
    }
    let patch = m.surface_patch(nearest.1);
    let index = patch.iter().copied().min().unwrap_or(nearest.1);
    // in part axes (see `in_alignment`)
    let tp = |v: Vec3| v3(to_part(arr(v)));
    let tris: Vec<[Vec3; 3]> = m
        .triangles
        .iter()
        .zip(&m.tri_face)
        .filter(|(_, f)| patch.contains(&(**f as usize)))
        .filter_map(|(t, _)| m.tri(t))
        .map(|t| t.map(tp))
        .collect();
    if tris.is_empty() {
        return Err(tr!("the face has no triangles").into());
    }
    let kind = match solidcraft_kernel::face_surface(&b.body, nearest.1) {
        Some(solidcraft_kernel::FaceSurface::Cylinder { origin, axis, radius }) => bore_kind(tp(origin), tp(axis.into()), radius, &tris)?,
        Some(solidcraft_kernel::FaceSurface::Plane { normal, .. }) => plane_kind(tp(normal.into()), &tris)?,
        _ => {
            // no exact surface: a planar mesh patch still counts as a plane
            let n = tri_normal(tris.first().copied().unwrap_or_default());
            if tris.iter().all(|t| dotv(tri_normal(*t), n) > 1.0 - 1e-6) {
                plane_kind(n, &tris)?
            } else {
                return Err(tr!("only planes and bores (cylinders) are supported").into());
            }
        }
    };
    Ok(FaceInfo { body: r.body.clone(), index, kind, tris })
}

fn tri_normal(t: [Vec3; 3]) -> Vec3 {
    let n = crossv(sub(t[1], t[0]), sub(t[2], t[0]));
    let l = dotv(n, n).sqrt();
    if l > 0.0 { Vec3 { x: n.x / l, y: n.y / l, z: n.z / l } } else { Vec3::default() }
}

fn bore_kind(origin: Vec3, axis: Vec3, radius: f64, tris: &[[Vec3; 3]]) -> Result<FaceKind, String> {
    if !(axis.z.abs() > 0.9995) {
        return Err(tr!("only vertical bores (axis Z) are supported").into());
    }
    if !(radius > 0.0 && radius.is_finite()) {
        return Err(tr!("cylinder without a valid radius").into());
    }
    // a bore: the face normal points towards the axis
    let mut inward = 0.0;
    let (mut zmin, mut zmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for t in tris {
        let c = Vec3 { x: (t[0].x + t[1].x + t[2].x) / 3.0, y: (t[0].y + t[1].y + t[2].y) / 3.0, z: (t[0].z + t[1].z + t[2].z) / 3.0 };
        let s = dotv(sub(c, origin), axis);
        let on_axis = Vec3 { x: origin.x + axis.x * s, y: origin.y + axis.y * s, z: origin.z + axis.z * s };
        let area = dotv(crossv(sub(t[1], t[0]), sub(t[2], t[0])), crossv(sub(t[1], t[0]), sub(t[2], t[0]))).sqrt();
        inward += dotv(tri_normal(*t), sub(on_axis, c)) * area;
        for p in t {
            zmin = zmin.min(p.z);
            zmax = zmax.max(p.z);
        }
    }
    if inward <= 0.0 {
        return Err(tr!("outer cylinders (pins) are not supported yet, only bores").into());
    }
    // axis position at the top
    let s = (zmax - origin.z) / axis.z;
    Ok(FaceKind::Bore {
        x: round(origin.x + axis.x * s),
        y: round(origin.y + axis.y * s),
        diameter: round(2.0 * radius),
        top: round(zmax),
        depth: round(zmax - zmin),
    })
}

fn plane_kind(normal: Vec3, tris: &[[Vec3; 3]]) -> Result<FaceKind, String> {
    let n = Normal::from_vector(arr(normal)).ok_or(tr!("only planes parallel to the axes (normal ±X, ±Y, ±Z) are supported"))?;
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for p in tris.iter().flatten() {
        for (i, c) in arr(*p).into_iter().enumerate() {
            if let (Some(lo), Some(hi)) = (min.get_mut(i), max.get_mut(i)) {
                *lo = lo.min(c);
                *hi = hi.max(c);
            }
        }
    }
    // exactly flat along the normal
    let i = usize::from(n.axis() - 1);
    if let (Some(lo), Some(hi)) = (min.get(i).copied(), max.get(i).copied()) {
        let mid = (lo + hi) / 2.0;
        if let (Some(a), Some(b)) = (min.get_mut(i), max.get_mut(i)) {
            (*a, *b) = (mid, mid);
        }
    }
    Ok(FaceKind::Plane { normal: n, min: min.map(round), max: max.map(round) })
}

/// Mesh coordinates to the micrometre (the tessellation leaves 1e-14 noise).
fn round(x: f64) -> f64 {
    let r = (x * 1e6).round() / 1e6;
    if r == 0.0 { 0.0 } else { r }
}

/// The local axes of a plane with this normal: X and Y the next two model axes in cyclic order
/// (swapped on a plane facing a negative axis, so the system stays right-handed with Z out of the
/// material).
fn plane_axes(n: Normal) -> (Vec3, Vec3, Vec3) {
    let e = |i: usize| match i % 3 {
        0 => Vec3 { x: 1.0, y: 0.0, z: 0.0 },
        1 => Vec3 { x: 0.0, y: 1.0, z: 0.0 },
        _ => Vec3 { x: 0.0, y: 0.0, z: 1.0 },
    };
    let i = usize::from(n.axis() - 1);
    let (a, b) = (e(i + 1), e(i + 2));
    let positive = n.vector().get(i).is_some_and(|c| *c > 0.0);
    let (x, y) = if positive { (a, b) } else { (b, a) };
    (x, y, v3(n.vector()))
}

/// Gap between a probing circle and the face's edges (probe sphere and clearance).
const EDGE_MARGIN: f64 = 2.0;

/// An axis-parallel plane face in its own 2D coordinates (its local axes, [`plane_axes`]).
struct Flat {
    ax: Vec3,
    ay: Vec3,
    nz: Vec3,
    level: f64,
    tris: Vec<[(f64, f64); 3]>,
    umin: f64,
    umax: f64,
    vmin: f64,
    vmax: f64,
}

impl Flat {
    fn of(info: &FaceInfo) -> Result<Flat, String> {
        let FaceKind::Plane { normal, min, .. } = &info.kind else { return Err(tr!("not a plane").into()) };
        let (ax, ay, nz) = plane_axes(*normal);
        let level = dotv(v3(*min), nz);
        let tris: Vec<[(f64, f64); 3]> = info.tris.iter().map(|t| t.map(|p| (dotv(p, ax), dotv(p, ay)))).collect();
        let (mut umin, mut umax, mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for t in &tris {
            for (u, v) in t {
                (umin, umax, vmin, vmax) = (umin.min(*u), umax.max(*u), vmin.min(*v), vmax.max(*v));
            }
        }
        if !(umax > umin && vmax > vmin) {
            return Err(tr!("the face has no extent").into());
        }
        Ok(Flat { ax, ay, nz, level, tris, umin, umax, vmin, vmax })
    }
    fn inside(&self, u: f64, v: f64) -> bool {
        self.tris.iter().any(|[a, b, c]| {
            let d = (b.1 - c.1) * (a.0 - c.0) + (c.0 - b.0) * (a.1 - c.1);
            if d.abs() < 1e-12 {
                return false;
            }
            let l1 = ((b.1 - c.1) * (u - c.0) + (c.0 - b.0) * (v - c.1)) / d;
            let l2 = ((c.1 - a.1) * (u - c.0) + (a.0 - c.0) * (v - c.1)) / d;
            let e = -1e-9;
            l1 >= e && l2 >= e && 1.0 - l1 - l2 >= e
        })
    }
    /// The whole disc of radius `r` around (u, v) lies on the face.
    fn disc_inside(&self, u: f64, v: f64, r: f64) -> bool {
        self.inside(u, v)
            && (0..16).all(|k| {
                let a = k as f64 * std::f64::consts::TAU / 16.0;
                self.inside(u + r * a.cos(), v + r * a.sin())
            })
    }
    fn size(&self) -> f64 {
        (self.umax - self.umin).min(self.vmax - self.vmin)
    }
    fn middle(&self) -> (f64, f64) {
        ((self.umin + self.umax) / 2.0, (self.vmin + self.vmax) / 2.0)
    }
    fn to3(&self, u: f64, v: f64) -> [f64; 3] {
        let (ax, ay, nz, l) = (self.ax, self.ay, self.nz, self.level);
        arr(Vec3 { x: ax.x * u + ay.x * v + nz.x * l, y: ax.y * u + ay.y * v + nz.y * l, z: ax.z * u + ay.z * v + nz.z * l })
    }
}

/// Circular probing paths on a plane face, spread over it and clear of its holes and edges.
pub fn circles(info: &FaceInfo, want: usize) -> Result<Vec<([f64; 3], f64)>, String> {
    let f = Flat::of(info)?;
    let size = f.size();
    // a narrow face (a pocket wall) keeps a smaller margin from its edges
    let margin = (size * 0.2).clamp(1.0, EDGE_MARGIN);
    if !(size > 2.0 * margin + 2.0) {
        return Err(tr!("the face is too narrow for a circular path").into());
    }
    let ring_ok = |u: f64, v: f64, r: f64| {
        (0..32).all(|k| {
            let a = k as f64 * std::f64::consts::TAU / 32.0;
            [r - margin, r, r + margin].iter().all(|rr| {
                let rr = rr.max(0.0);
                f.inside(u + rr * a.cos(), v + rr * a.sin())
            })
        })
    };
    // even, so the face's middle lines are on the grid
    const GRID: usize = 16;
    let (cu, cv) = f.middle();
    let mut best: Vec<(f64, f64, f64)> = Vec::new();
    for d in [20.0, 16.0, 12.0, 10.0, 8.0, 6.0, 5.0, 4.0, 3.0, 2.0] {
        if d > 0.8 * size {
            continue;
        }
        let r = d / 2.0;
        let mut valid = Vec::new();
        for i in 0..=GRID {
            for j in 0..=GRID {
                let u = f.umin + (f.umax - f.umin) * i as f64 / GRID as f64;
                let v = f.vmin + (f.vmax - f.vmin) * j as f64 / GRID as f64;
                if ring_ok(u, v, r) {
                    valid.push((u, v));
                }
            }
        }
        // spread: start far from the middle, then always the one farthest from those chosen
        let mut chosen: Vec<(f64, f64)> = Vec::new();
        let dist = |a: (f64, f64), b: (f64, f64)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        while chosen.len() < want {
            let next = valid.iter().copied().filter(|p| chosen.iter().all(|c| dist(*c, *p) >= d + 2.0 * margin)).max_by(|a, b| {
                let score = |p: (f64, f64)| {
                    if chosen.is_empty() { dist(p, (cu, cv)) } else { chosen.iter().map(|c| dist(*c, p)).fold(f64::INFINITY, f64::min) }
                };
                score(*a).total_cmp(&score(*b))
            });
            match next {
                Some(p) => chosen.push(p),
                None => break,
            }
        }
        if chosen.len() > best.len() {
            best = chosen.iter().map(|(u, v)| (*u, *v, d)).collect();
        }
        if best.len() >= want {
            break;
        }
    }
    if best.is_empty() {
        return Err(tr!("there is no room for a circular path on the face").into());
    }
    Ok(best.into_iter().map(|(u, v, d)| (f.to3(u, v), d)).collect())
}

/// `count` single points spread over an axis-parallel plane face, clear of its edges and holes
/// (the probe sphere must fit): first the one farthest from the middle, then always the one
/// farthest from those chosen.
pub fn spread_points(info: &FaceInfo, count: usize) -> Result<Vec<[f64; 3]>, String> {
    let f = Flat::of(info)?;
    let margin = (f.size() * 0.25).clamp(0.5, 3.0);
    const GRID: usize = 24;
    let mut valid = Vec::new();
    for i in 0..=GRID {
        for j in 0..=GRID {
            let u = f.umin + (f.umax - f.umin) * i as f64 / GRID as f64;
            let v = f.vmin + (f.vmax - f.vmin) * j as f64 / GRID as f64;
            if f.disc_inside(u, v, margin) {
                valid.push((u, v));
            }
        }
    }
    if valid.is_empty() {
        return Err(tr!("there is no room for probing points on the face").into());
    }
    let (cu, cv) = f.middle();
    let d2 = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2);
    let mut chosen: Vec<(f64, f64)> = Vec::new();
    while chosen.len() < count.min(valid.len()) {
        let score = |p: (f64, f64)| if chosen.is_empty() { d2(p, (cu, cv)) } else { chosen.iter().map(|c| d2(*c, p)).fold(f64::INFINITY, f64::min) };
        match valid.iter().copied().max_by(|a, b| score(*a).total_cmp(&score(*b))) {
            Some(p) if chosen.is_empty() || score(p) > 1e-6 => chosen.push(p),
            _ => break,
        }
    }
    Ok(chosen.into_iter().map(|(u, v)| f.to3(u, v).map(round)).collect())
}

/// The outward normal of a plane face, inclined ones too (for angles).
pub fn plane_normal(state: &solidcraft_doc::ModelState, r: &FaceRef) -> Result<[f64; 3], String> {
    let b = state.body(&r.body).ok_or_else(|| tr!("Body {body} does not exist (any more)", body = format!("{:?}", r.body)))?;
    if !r.point.iter().all(|c| c.is_finite()) {
        return Err(tr!("invalid point").into());
    }
    let m = b.mesh();
    let p = v3(r.point);
    let (d2, face, tri) = m
        .triangles
        .iter()
        .zip(&m.tri_face)
        .filter_map(|(t, f)| m.tri(t).map(|tri| (dist2_triangle(p, &tri), *f as usize, tri)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .ok_or_else(|| tr!("Body {body} has no faces", body = format!("{:?}", r.body)))?;
    if d2 > (m.bounds().diagonal().max(1.0) * 1e-3).powi(2) {
        return Err(tr!("there is no face of {body} at {point}", body = format!("{:?}", r.body), point = format!("{:?}", r.point)));
    }
    match solidcraft_kernel::face_surface(&b.body, face) {
        Some(solidcraft_kernel::FaceSurface::Plane { normal, .. }) => Ok(to_part(arr(normal.into()))),
        Some(_) => Err(tr!("choose a plane").into()),
        None => Ok(to_part(arr(tri_normal(tri)))),
    }
}

// ------------------------------------------------------------------------------------------
// Names
// ------------------------------------------------------------------------------------------

/// A number for names and descriptions: up to three decimals, with the language's decimal
/// separator (German: comma).
pub fn num(x: f64) -> String {
    let s = format!("{:.3}", if x.abs() < 5e-4 { 0.0 } else { x });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    match crate::inspection_text::language() {
        crate::inspection_text::Lang::De => s.replace('.', ","),
        crate::inspection_text::Lang::En => s.to_string(),
    }
}

/// A readable default name.
pub fn default_name(info: &FaceInfo) -> String {
    match &info.kind {
        FaceKind::Bore { x, y, diameter, .. } => tr!("Cylinder Ø{d} X{x} Y{y}", d = num(*diameter), x = num(*x), y = num(*y)),
        FaceKind::Plane { normal, min, .. } => {
            let i = usize::from(normal.axis() - 1);
            let axis = ["X", "Y", "Z"].get(i).copied().unwrap_or("?");
            tr!("Plane {normal} {axis}{at}", normal = normal.label(), axis = axis, at = num(min.get(i).copied().unwrap_or(0.0)))
        }
    }
}

/// A bore's default name as a circle (measured as a circle below its opening).
pub fn circle_name(info: &FaceInfo) -> String {
    match &info.kind {
        FaceKind::Bore { x, y, diameter, .. } => tr!("Circle Ø{d} X{x} Y{y}", d = num(*diameter), x = num(*x), y = num(*y)),
        FaceKind::Plane { .. } => default_name(info),
    }
}

/// Short description of a face (for lists).
pub fn describe(info: &FaceInfo) -> Value {
    match &info.kind {
        FaceKind::Bore { x, y, diameter, top, depth } => {
            json!({"type": "bore", "x": x, "y": y, "diameter": diameter, "top": top, "depth": depth, "body": info.body})
        }
        FaceKind::Plane { normal, min, max } => json!({"type": "plane", "normal": normal.label(), "min": min, "max": max, "body": info.body}),
    }
}

fn need<'a, T>(v: &'a Option<T>, what: &str) -> Result<&'a T, String> {
    v.as_ref().ok_or_else(|| tr!("{what} is missing", what = what))
}

// ------------------------------------------------------------------------------------------
// The alignment's coordinate system
// ------------------------------------------------------------------------------------------

/// The part coordinate system an alignment makes (unit axes, in model coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub z: [f64; 3],
}

impl Frame {
    /// A model point in this coordinate system.
    pub fn local(&self, p: [f64; 3]) -> [f64; 3] {
        let d = sub(v3(p), v3(self.origin));
        [dotv(d, v3(self.x)), dotv(d, v3(self.y)), dotv(d, v3(self.z))]
    }
    pub fn to_json(&self) -> Value {
        json!({"origin": self.origin.map(round), "x": self.x.map(round), "y": self.y.map(round), "z": self.z.map(round)})
    }
    /// A frame in part axes, in model coordinates ([`to_model`]).
    fn to_model(self) -> Frame {
        Frame { origin: to_model(self.origin).map(round), x: to_model(self.x), y: to_model(self.y), z: to_model(self.z) }
    }
    /// A frame in model coordinates, in part axes ([`to_part`]).
    fn to_part(self) -> Frame {
        Frame { origin: to_part(self.origin), x: to_part(self.x), y: to_part(self.y), z: to_part(self.z) }
    }
}

/// The coordinate system of a complete alignment.
/// Are `a` and `b` the wall of one bore (both halves of a cylinder, or one picked twice)?
/// Two picks in one bore are that bore, not the middle of two sides.
pub fn same_bore(model: &solidcraft_doc::ModelState, a: &FaceRef, b: &FaceRef) -> bool {
    match (analyze(model, a).map(|i| i.kind), analyze(model, b).map(|i| i.kind)) {
        (Ok(FaceKind::Bore { x, y, diameter, .. }), Ok(FaceKind::Bore { x: x2, y: y2, diameter: d2, .. })) => {
            (x - x2).hypot(y - y2) < 1e-4 && (diameter - d2).abs() < 1e-4
        }
        _ => false,
    }
}

pub fn frame(model: &solidcraft_doc::ModelState, a: &Alignment) -> Result<Frame, String> {
    in_alignment(model, a, || frame_part(model, a).map(|f| f.to_model()))
}

/// The alignment's coordinate system in part axes (inside [`in_alignment`]).
fn frame_part(model: &solidcraft_doc::ModelState, a: &Alignment) -> Result<Frame, String> {
    let primary = analyze(model, need(&a.primary, tr!("the primary plane"))?)?;
    let z0 = match primary.kind {
        // a face the part stands on: Z points away from it, into the part
        FaceKind::Plane { normal: Normal::MinusZ, min, .. } if a.clamp.is_some() => min[2],
        FaceKind::Plane { normal: Normal::PlusZ, min, .. } if a.clamp.is_none() => min[2],
        FaceKind::Plane { .. } => return Err(tr!("Primary plane: the face must look up (+Z)").into()),
        FaceKind::Bore { .. } => return Err(tr!("Primary plane: choose a plane, not a bore").into()),
    };
    let z = Vec3 { x: 0.0, y: 0.0, z: 1.0 };
    let x = match need(&a.direction, tr!("the direction"))? {
        DirectionRef::Side { face } => match analyze(model, face)?.kind {
            FaceKind::Plane { normal, .. } if normal.axis() != 3 => Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            FaceKind::Plane { .. } => return Err(tr!("Direction: choose a side face (±X/±Y), not the top or bottom").into()),
            FaceKind::Bore { .. } => return Err(tr!("Direction: choose a side face or two bores").into()),
        },
        DirectionRef::Bores { first, second } => {
            let at = |r: &FaceRef| match analyze(model, r)?.kind {
                FaceKind::Bore { x, y, .. } => Ok((x, y)),
                FaceKind::Plane { .. } => Err(tr!("Direction: choose two bores (or a side face)").to_string()),
            };
            let (a, b) = (at(first)?, at(second)?);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l = (dx * dx + dy * dy).sqrt();
            if !(l > 1e-6) {
                return Err(tr!("Direction: the two bores lie on top of each other").into());
            }
            Vec3 { x: dx / l, y: dy / l, z: 0.0 }
        }
    };
    let y = crossv(z, x);
    let along = |o: &OriginRef, axis: Vec3, letter: &str| -> Result<f64, String> {
        let side = |r: &FaceRef| -> Result<f64, String> {
            match analyze(model, r)?.kind {
                FaceKind::Plane { normal, min, .. } => {
                    if dotv(v3(normal.vector()), axis).abs() < 0.999 {
                        Err(tr!("Origin {letter}: the side face must stand across the {letter} axis", letter = letter))
                    } else {
                        Ok(dotv(v3(min), axis))
                    }
                }
                FaceKind::Bore { .. } => Err(tr!("Origin {letter}: choose a side face", letter = letter)),
            }
        };
        match o {
            OriginRef::Side { face } => side(face),
            OriginRef::Middle { first, second } => {
                let (a, b) = (side(first)?, side(second)?);
                if (a - b).abs() < 1e-6 {
                    return Err(tr!("Origin {letter}: choose two opposite side faces", letter = letter));
                }
                Ok((a + b) / 2.0)
            }
            OriginRef::Bore { face } => match analyze(model, face)?.kind {
                FaceKind::Bore { x, y, top, .. } => Ok(dotv(Vec3 { x, y, z: top }, axis)),
                FaceKind::Plane { .. } => Err(tr!("Origin {letter}: choose a bore or a side face", letter = letter)),
            },
        }
    };
    let ox = along(need(&a.origin_x, tr!("the origin X"))?, x, "X")?;
    let oy = along(need(&a.origin_y, tr!("the origin Y"))?, y, "Y")?;
    let o = Vec3 { x: x.x * ox + y.x * oy + z.x * z0, y: x.y * ox + y.y * oy + z.y * z0, z: x.z * ox + y.z * oy + z.z * z0 };
    Ok(Frame { origin: arr(o).map(round), x: arr(x), y: arr(y), z: arr(z) })
}

/// The alignment as lines of text, one per step (`None`: not set).
pub fn alignment_text(model: &solidcraft_doc::ModelState, a: &Alignment) -> Value {
    let face = |r: &FaceRef| analyze(model, r).map_or_else(|e| format!("⚠ {e}"), |f| default_name(&f));
    // a bore of the alignment, as it is measured: its cylinder's axis or its circle's centre
    let bore = |r: &FaceRef| match a.bores {
        AlignBores::Cylinder => tr!("Axis {name}", name = face(r)),
        AlignBores::Circle => tr!("Centre {name}", name = analyze(model, r).map_or_else(|e| format!("⚠ {e}"), |f| circle_name(&f))),
    };
    let origin = |o: &Option<OriginRef>| {
        o.as_ref().map(|o| match o {
            OriginRef::Side { face: f } => face(f),
            OriginRef::Middle { first, second } => tr!("Middle {a} / {b}", a = face(first), b = face(second)),
            OriginRef::Bore { face: f } => bore(f),
        })
    };
    json!({
        "primary": a.primary.as_ref().map(face),
        "direction": a.direction.as_ref().map(|d| match d {
            DirectionRef::Side { face: f } => face(f),
            DirectionRef::Bores { first, second } => match a.bores {
                AlignBores::Cylinder => format!("{} → {}", face(first), face(second)),
                AlignBores::Circle => format!("{} → {}", bore(first), bore(second)),
            },
        }),
        "origin_x": origin(&a.origin_x),
        "origin_y": origin(&a.origin_y),
    })
}

/// The measured elements the alignment fixes, one per face (a face used twice is listed once):
/// `{role, name, kind (plane|cylinder|circle), command (its measurement's dialog), face,
/// strategy, characteristic}`. `characteristic` is the measurement of that face (a plane
/// measurement, cylinder measurement or circle) whose strategy the alignment uses, else null:
/// then the setup's default strategy applies.
pub fn alignment_elements(model: &solidcraft_doc::ModelState, setup: &Setup) -> Value {
    in_alignment(model, &setup.alignment, || alignment_elements_part(model, setup))
}

fn alignment_elements_part(model: &solidcraft_doc::ModelState, setup: &Setup) -> Value {
    let a = &setup.alignment;
    let mut refs: Vec<(&str, &FaceRef)> = Vec::new();
    if let Some(f) = &a.primary {
        refs.push(("primary", f));
    }
    match &a.direction {
        Some(DirectionRef::Side { face }) => refs.push(("direction", face)),
        Some(DirectionRef::Bores { first, second }) => {
            refs.push(("direction", first));
            refs.push(("direction", second));
        }
        None => {}
    }
    for (role, o) in [("origin_x", &a.origin_x), ("origin_y", &a.origin_y)] {
        match o {
            Some(OriginRef::Side { face } | OriginRef::Bore { face }) => refs.push((role, face)),
            Some(OriginRef::Middle { first, second }) => {
                refs.push((role, first));
                refs.push((role, second));
            }
            None => {}
        }
    }
    let mut out: Vec<(FaceInfo, Value)> = Vec::new();
    for (role, r) in refs {
        let Ok(info) = analyze(model, r) else { continue };
        if let Some((_, v)) = out.iter_mut().find(|(i, _)| i.same(&info)) {
            // the same face in another role: one row, both roles
            let mut roles: Vec<String> = v["roles"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            if !roles.iter().any(|x| x == role) {
                roles.push(role.to_string());
                v["role"] = json!(roles.iter().map(|r| role_label(r)).collect::<Vec<_>>().join(", "));
                v["roles"] = json!(roles);
            }
            continue;
        }
        let circle = matches!(info.kind, FaceKind::Bore { .. }) && a.bores == AlignBores::Circle;
        let (kind, command) = match info.kind {
            FaceKind::Plane { .. } => ("plane", "cmm.plane"),
            FaceKind::Bore { .. } if circle => ("circle", "cmm.circle"),
            FaceKind::Bore { .. } => ("cylinder", "cmm.cylinder"),
        };
        // the measurement of this face that the alignment takes its strategy from
        let own = setup.characteristics.iter().find(|c| {
            let fits = matches!(
                (kind, &c.kind),
                ("plane", CharacteristicKind::Plane { .. })
                    | ("cylinder", CharacteristicKind::Cylinder { .. })
                    | ("circle", CharacteristicKind::Circle { .. })
            );
            fits && c.kind.faces().first().and_then(|f| analyze(model, f).ok()).is_some_and(|i| i.same(&info))
        });
        let strategy = match own.map(|c| &c.kind) {
            Some(CharacteristicKind::Plane { strategy: PlaneProbing::Circles, .. }) => tr!("Circular paths").to_string(),
            Some(CharacteristicKind::Plane { picks, .. }) if !picks.is_empty() => tr!("{n} picked points", n = picks.len()),
            Some(CharacteristicKind::Plane { points, .. }) => tr!("{n} points", n = points),
            Some(CharacteristicKind::Cylinder { strategy: CylinderProbing::Scan, paths, .. }) => tr!("Scan, {paths} paths", paths = paths),
            Some(CharacteristicKind::Cylinder { strategy: CylinderProbing::CirclePoints, paths, points, .. }) => {
                tr!("{paths} paths × {n} points", paths = paths, n = points)
            }
            Some(CharacteristicKind::Cylinder { picks, .. }) => tr!("{n} picked points", n = picks.len()),
            Some(CharacteristicKind::Circle { points, height, .. }) => {
                tr!("{n} points, {h} mm below the opening", n = points, h = num(*height))
            }
            _ => match kind {
                "plane" if setup.plane_probing == PlaneProbing::Circles => tr!("Setup: circular paths").to_string(),
                "plane" => tr!("Setup: 4 points").to_string(),
                "circle" => tr!("Default: 4 points, 2 mm below the opening").to_string(),
                _ => tr!("Default: scan, 2 paths").to_string(),
            },
        };
        let name = if circle { circle_name(&info) } else { default_name(&info) };
        // where the alignment probes it: as its own measurement says, else as by default
        let shown = own.cloned().unwrap_or_else(|| Characteristic {
            id: 0,
            name: String::new(),
            kind: match kind {
                "plane" => CharacteristicKind::Plane { face: r.clone(), strategy: setup.plane_probing, points: 4, picks: Vec::new() },
                "circle" => CharacteristicKind::Circle { face: r.clone(), points: 4, height: 2.0, position: false },
                _ => CharacteristicKind::Cylinder {
                    face: r.clone(),
                    strategy: CylinderProbing::Scan,
                    paths: 2,
                    points: 7,
                    height: 2.0,
                    picks: Vec::new(),
                },
            },
            tol: 0.05,
            safety_plane: None,
            probe_file: None,
        });
        let mut markers = characteristic_markers(model, setup, &shown);
        let (mut name, mut strategy) = (name, strategy);
        // the clamping plate: probed beside the part, not on the face
        if role == "primary"
            && let Some(clamp) = &a.clamp
        {
            let plate = frame_part(model, a).and_then(|fr| clamp_element(model, setup, clamp, &fr, String::new()));
            if let Ok(el) = plate {
                let mut m = element_markers(&el);
                markers_to_model(&mut m);
                strategy = tr!("{n} points on the clamping plate", n = m.len());
                markers = json!(m);
            }
            name = tr!("Clamping plate ({face})", face = name);
        }
        let v = json!({
            "role": role_label(role),
            "roles": [role],
            "name": name,
            "kind": kind,
            "command": command,
            "face": r,
            "strategy": strategy,
            "characteristic": own.map(|c| c.id),
            "probe_file": own.and_then(|c| c.probe_file.clone()),
            "markers": markers,
        });
        out.push((info, v));
    }
    Value::Array(out.into_iter().map(|(_, v)| v).collect())
}

/// What an alignment role (`primary`, `direction`, `origin_x`, `origin_y`) fixes, as shown.
pub fn role_label(role: &str) -> &'static str {
    match role {
        "primary" => tr!("Primary plane"),
        "direction" => tr!("Direction"),
        "origin_x" => tr!("Origin X"),
        "origin_y" => tr!("Origin Y"),
        _ => "",
    }
}

// ------------------------------------------------------------------------------------------
// Characteristics
// ------------------------------------------------------------------------------------------

/// The name of a characteristic kind (as the tree lists it).
pub fn kind_label(k: &CharacteristicKind) -> &'static str {
    match k {
        CharacteristicKind::Circle { .. } => tr!("Circle"),
        CharacteristicKind::Cylinder { .. } => tr!("Cylinder measurement"),
        CharacteristicKind::Plane { .. } => tr!("Plane measurement"),
        CharacteristicKind::Distance { .. } => tr!("Distance"),
        CharacteristicKind::Angle { .. } => tr!("Angle"),
        CharacteristicKind::Point { .. } => tr!("Point"),
    }
}

fn bore_of(model: &solidcraft_doc::ModelState, r: &FaceRef) -> Result<FaceInfo, String> {
    let f = analyze(model, r)?;
    match f.kind {
        FaceKind::Bore { .. } => Ok(f),
        FaceKind::Plane { .. } => Err(tr!("choose a bore, not a plane").into()),
    }
}

/// A face's position along an axis, and whether it may be measured along it.
fn coordinate(f: &FaceInfo, axis: Axis) -> Option<f64> {
    let i = match axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    };
    match &f.kind {
        FaceKind::Plane { normal, min, .. } => (usize::from(normal.axis() - 1) == i).then(|| min.get(i).copied()).flatten(),
        FaceKind::Bore { x, y, top, .. } => Some([*x, *y, *top].get(i).copied().unwrap_or(0.0)),
    }
}

fn plane_axis(f: &FaceInfo) -> Option<Axis> {
    match &f.kind {
        FaceKind::Plane { normal, .. } => [Axis::X, Axis::Y, Axis::Z].get(usize::from(normal.axis() - 1)).copied(),
        FaceKind::Bore { .. } => None,
    }
}

/// The axis and nominal of a distance between two faces (planes along their normal, bores
/// along X, Y or Z).
pub fn distance(a: &FaceInfo, b: &FaceInfo, axis: Option<Axis>) -> Result<(Axis, f64), String> {
    if a.same(b) {
        return Err(tr!("choose two different elements").into());
    }
    let axis = match (axis, plane_axis(a), plane_axis(b)) {
        (Some(x), _, _) => x,
        (None, Some(p), Some(q)) if p != q => return Err(tr!("the two planes are not parallel").into()),
        (None, Some(p), _) | (None, None, Some(p)) => p,
        (None, None, None) => {
            let (ca, cb) = (
                coordinate(a, Axis::X).unwrap_or(0.0) - coordinate(b, Axis::X).unwrap_or(0.0),
                coordinate(a, Axis::Y).unwrap_or(0.0) - coordinate(b, Axis::Y).unwrap_or(0.0),
            );
            if ca.abs() >= cb.abs() { Axis::X } else { Axis::Y }
        }
    };
    match (coordinate(a, axis), coordinate(b, axis)) {
        (Some(p), Some(q)) => Ok((axis, round((q - p).abs()))),
        _ => Err(tr!("a plane can only be measured across it, not along {axis}", axis = axis.letter())),
    }
}

/// The angle between two planes' normals (degrees).
pub fn angle(model: &solidcraft_doc::ModelState, first: &FaceRef, second: &FaceRef) -> Result<f64, String> {
    let (a, b) = (v3(plane_normal(model, first)?), v3(plane_normal(model, second)?));
    let c = dotv(a, b) / (dotv(a, a).sqrt() * dotv(b, b).sqrt());
    if !c.is_finite() {
        return Err(tr!("the face normal is missing").into());
    }
    let deg = c.clamp(-1.0, 1.0).acos().to_degrees();
    if deg < 1e-6 || 180.0 - deg < 1e-6 {
        return Err(tr!("the two planes are parallel").into());
    }
    Ok(round(deg))
}

fn plane_of(model: &solidcraft_doc::ModelState, r: &FaceRef) -> Result<FaceInfo, String> {
    let f = analyze(model, r)?;
    match f.kind {
        FaceKind::Plane { .. } => Ok(f),
        FaceKind::Bore { .. } => Err(tr!("choose a plane, not a bore").into()),
    }
}

/// Checks points picked on a face: each lies on it (`at least` of them), rounded.
pub fn check_picks(model: &solidcraft_doc::ModelState, face: &FaceInfo, picks: &[[f64; 3]], at_least: usize) -> Result<Vec<[f64; 3]>, String> {
    if picks.len() > MAX_PICKS {
        return Err(tr!("at most {n} points", n = MAX_PICKS));
    }
    if picks.len() < at_least {
        return Err(tr!("pick at least {n} points on the face ({picked} picked)", n = at_least, picked = picks.len()));
    }
    for (i, p) in picks.iter().enumerate() {
        let on = analyze(model, &FaceRef { body: face.body.clone(), point: *p }).is_ok_and(|f| f.same(face));
        if !on {
            return Err(tr!("point {i} is not on the chosen face", i = i + 1));
        }
    }
    Ok(picks.iter().map(|p| to_part(*p).map(round)).collect())
}

/// Least number of points picked for a plane or cylinder measured with single points.
pub const MIN_PICKS: usize = 4;

/// The height of a circle in a bore: `height` below its opening, inside the bore.
fn circle_center(f: &FaceInfo, height: f64) -> Result<[f64; 3], String> {
    let FaceKind::Bore { x, y, top, depth, .. } = f.kind else { return Err(tr!("choose a bore").into()) };
    if !(height.is_finite() && height > 0.0 && height < depth) {
        return Err(tr!("Distance to the plane: between 0 and the bore depth {d} mm", d = num(depth)));
    }
    Ok([x, y, round(top - height)])
}

/// The gap of a cylinder's first and last path from its ends must leave room for the paths.
fn check_margin(f: &FaceInfo, height: f64, paths: u32) -> Result<(), String> {
    let FaceKind::Bore { depth, .. } = f.kind else { return Err(tr!("choose a bore").into()) };
    let room = if paths > 1 { depth - 2.0 * height } else { depth - height };
    if !(height.is_finite() && height >= 0.0 && room > 0.0) {
        return Err(tr!("Distance to the plane {h} mm does not fit the bore depth {d} mm", h = num(height), d = num(depth)));
    }
    Ok(())
}

/// What a characteristic measures, from the model: its automatic name and nominal text.
fn measure(model: &solidcraft_doc::ModelState, frame: Option<&Frame>, c: &Characteristic) -> Result<(String, String, Value), String> {
    let tol = num(c.tol);
    match &c.kind {
        CharacteristicKind::Circle { face, points, height, position } => {
            let f = bore_of(model, face)?;
            let FaceKind::Bore { x, y, diameter, .. } = f.kind else { return Err(tr!("choose a bore").into()) };
            let center = circle_center(&f, *height)?;
            if !(3..=100).contains(points) {
                return Err(tr!("Points: 3 to 100").into());
            }
            let mut text = tr!("Ø{d} ±{tol} · {n} points · {h} mm below the plane", d = num(diameter), tol = tol, n = points, h = num(*height));
            if *position {
                match frame {
                    Some(fr) => {
                        let l = fr.local(center);
                        text.push_str(&format!(" · X{} Y{}", num(l[0]), num(l[1])));
                    }
                    None => text.push_str(tr!(" · Position: set the alignment first")),
                }
            }
            Ok((tr!("Circle Ø{d} X{x} Y{y}", d = num(diameter), x = num(x), y = num(y)), text, json!(diameter)))
        }
        CharacteristicKind::Cylinder { face, strategy, paths, points, height, picks } => {
            let f = bore_of(model, face)?;
            let FaceKind::Bore { diameter, depth, .. } = f.kind else { return Err(tr!("choose a bore").into()) };
            let how = match strategy {
                CylinderProbing::Scan => {
                    check_margin(&f, *height, *paths)?;
                    tr!("Scan, {paths} paths", paths = paths)
                }
                CylinderProbing::CirclePoints => {
                    check_margin(&f, *height, *paths)?;
                    if !(3..=100).contains(points) {
                        return Err(tr!("Points per path: 3 to 100").into());
                    }
                    tr!("{paths} paths × {n} points", paths = paths, n = points)
                }
                CylinderProbing::Points => {
                    let n = check_picks(model, &f, picks, MIN_PICKS)?.len();
                    tr!("{n} points (picked)", n = n)
                }
            };
            if !(1..=20).contains(paths) {
                return Err(tr!("Paths: 1 to 20").into());
            }
            Ok((
                default_name(&f),
                tr!("Ø{d} ±{tol} · depth {depth} · {how}", d = num(diameter), tol = tol, depth = num(depth), how = how),
                json!(diameter),
            ))
        }
        CharacteristicKind::Plane { face, strategy, points, picks } => {
            let f = plane_of(model, face)?;
            let how = match strategy {
                PlaneProbing::Points if !picks.is_empty() => tr!("{n} points (picked)", n = check_picks(model, &f, picks, MIN_PICKS)?.len()),
                PlaneProbing::Points => {
                    if !(3..=100).contains(points) {
                        return Err(tr!("Points: 3 to 100").into());
                    }
                    let n = spread_points(&f, *points as usize)?.len();
                    tr!("{n} points (spread)", n = n)
                }
                PlaneProbing::Circles => {
                    let want = if matches!(f.kind, FaceKind::Plane { normal: Normal::PlusZ | Normal::MinusZ, .. }) { 4 } else { 2 };
                    tr!("{n} circular paths", n = circles(&f, want)?.len())
                }
            };
            Ok((default_name(&f), how, Value::Null))
        }
        CharacteristicKind::Distance { from, to, axis } => {
            let (a, b) = (analyze(model, from)?, analyze(model, to)?);
            let (ax, d) = distance(&a, &b, *axis)?;
            Ok((
                tr!("Distance {a} – {b} {axis}", a = default_name(&a), b = default_name(&b), axis = ax.letter()),
                format!("{} ±{tol} ({})", num(d), ax.letter()),
                json!(d),
            ))
        }
        CharacteristicKind::Angle { first, second } => {
            let deg = angle(model, first, second)?;
            let name = |r: &FaceRef| analyze(model, r).map_or_else(|_| tr!("Plane").to_string(), |f| default_name(&f));
            Ok((tr!("Angle {a} – {b}", a = name(first), b = name(second)), format!("{}° ±{tol}°", num(deg)), json!(deg)))
        }
        CharacteristicKind::Point { points, check } => {
            let Some(first) = points.first() else { return Err(tr!("no points: pick points on the body").into()) };
            if points.len() > MAX_POINTS {
                return Err(tr!("at most {n} points", n = MAX_POINTS));
            }
            for (i, f) in points.iter().enumerate() {
                surface_normal(model, f).map_err(|e| tr!("Point {i}: {e}", i = i + 1, e = e))?;
            }
            let local: Vec<[f64; 3]> = points.iter().map(|f| to_part(f.point)).map(|q| frame.map_or(q, |fr| fr.local(q)).map(round)).collect();
            let how = match check {
                PointCheck::Normal => "normal",
                PointCheck::Coordinates => "X Y Z",
            };
            if let [p] = local.as_slice() {
                let q = to_part(first.point);
                let name = tr!("Point X{x} Y{y} Z{z}", x = num(q[0]), y = num(q[1]), z = num(q[2]));
                let text = match check {
                    PointCheck::Coordinates => format!("X{} Y{} Z{} ±{tol}", num(p[0]), num(p[1]), num(p[2])),
                    PointCheck::Normal => tr!("Deviation 0 ±{tol} (normal) · X{x} Y{y} Z{z}", tol = tol, x = num(p[0]), y = num(p[1]), z = num(p[2])),
                };
                return Ok((name, text, json!(p)));
            }
            Ok((
                tr!("Point group {n} points", n = local.len()),
                tr!("{n} points ±{tol} ({how})", n = local.len(), tol = tol, how = how),
                json!(local),
            ))
        }
    }
}

/// The name a characteristic is listed and exported under.
pub fn characteristic_name(model: &solidcraft_doc::ModelState, c: &Characteristic) -> String {
    if !c.name.trim().is_empty() {
        return c.name.trim().to_string();
    }
    measure(model, None, c).map_or_else(|_| format!("{} {}", kind_label(&c.kind), c.id), |(n, _, _)| n)
}

/// A characteristic for the tree and `cmm.state`.
pub fn characteristic_info(model: &solidcraft_doc::ModelState, frame: Option<&Frame>, c: &Characteristic) -> Value {
    // the frame comes in model coordinates; faces are analysed in part axes
    let frame_part = frame.map(|f| f.to_part());
    let frame = frame_part.as_ref();
    let faces: Vec<Value> = c.kind.faces().iter().map(|f| json!({"body": f.body, "point": f.point})).collect();
    let mut v = json!({
        "id": c.id,
        "type": serde_json::to_value(&c.kind).ok().and_then(|k| k.get("type").cloned()).unwrap_or(Value::Null),
        "command": c.kind.command(),
        "kind": kind_label(&c.kind),
        "given_name": c.name,
        "tol": c.tol,
        "faces": faces,
        "safety_plane": c.safety_plane,
        "probe_file": c.probe_file,
    });
    match measure(model, frame, c) {
        Ok((auto, text, value)) => {
            v["name"] = json!(if c.name.trim().is_empty() { auto } else { c.name.trim().to_string() });
            v["nominal"] = json!(text);
            v["value"] = value;
        }
        Err(e) => {
            v["name"] = json!(if c.name.trim().is_empty() { format!("{} {}", kind_label(&c.kind), c.id) } else { c.name.trim().to_string() });
            v["error"] = json!(e);
        }
    }
    v
}

// ------------------------------------------------------------------------------------------
// The neutral CMM program
// ------------------------------------------------------------------------------------------

/// Where an element's probe retracts to and which probe configuration measures it.
#[derive(Clone, Debug, PartialEq)]
struct How {
    safety: SafetyPlane,
    /// The probe file; `None`: the setup's.
    probe: Option<String>,
}

/// A face of the program: its name, and the safety plane and probe of its first use.
struct Entry {
    info: FaceInfo,
    name: String,
    how: How,
}

/// The faces of the program, each once, in order of first use, with their names.
struct Registry {
    faces: Vec<Entry>,
}

impl Registry {
    fn add(&mut self, info: FaceInfo, name: Option<&str>, how: &How) -> String {
        if let Some(e) = self.faces.iter().find(|e| e.info.same(&info)) {
            return e.name.clone();
        }
        let base = name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| default_name(&info), str::to_string);
        let n = self.unique(&base, &[]);
        self.faces.push(Entry { info, name: n.clone(), how: how.clone() });
        n
    }
    /// `base`, or `base (2)`, … not yet used here nor in `others`.
    fn unique(&self, base: &str, others: &[pg::Element]) -> String {
        let taken = |n: &str| self.faces.iter().any(|e| e.name == n) || others.iter().any(|e| e.name == n);
        let mut n = base.to_string();
        let mut k = 2;
        while taken(&n) && k < 10_000 {
            n = format!("{base} ({k})");
            k += 1;
        }
        n
    }
}

/// How the setup's faces are measured: its plane and cylinder measurements, by face (the
/// first one of a face counts).
struct Settings<'a> {
    faces: Vec<(FaceInfo, &'a Characteristic)>,
    plane_default: PlaneProbing,
    setup: &'a Setup,
}

impl<'a> Settings<'a> {
    /// `first` (a characteristic being edited) comes before the setup's own.
    fn of(model: &solidcraft_doc::ModelState, setup: &'a Setup, first: Option<&'a Characteristic>) -> Settings<'a> {
        let mut faces = Vec::new();
        for c in first.into_iter().chain(setup.characteristics.iter().filter(|x| first.is_none_or(|f| f.id != x.id))).take(MAX_CHARACTERISTICS) {
            if let CharacteristicKind::Plane { face, .. } | CharacteristicKind::Cylinder { face, .. } = &c.kind
                && let Ok(f) = analyze(model, face)
            {
                faces.push((f, c));
            }
        }
        Settings { faces, plane_default: setup.plane_probing, setup }
    }
    fn of_face(&self, info: &FaceInfo) -> Option<&'a Characteristic> {
        self.faces.iter().find(|(f, _)| f.same(info)).map(|(_, c)| *c)
    }
}

fn safety_label(s: SafetyPlane) -> Option<String> {
    s.label().map(str::to_string)
}

/// The paths of a cylinder scan along the depth: the heights of the circles.
fn path_heights(top: f64, depth: f64, margin: f64, paths: u32) -> Vec<f64> {
    let n = paths.clamp(1, 20);
    if n == 1 {
        return vec![round(top - margin)];
    }
    let (first, last) = (top - margin, top - depth + margin);
    (0..n).map(|i| round(first + (last - first) * f64::from(i) / f64::from(n - 1))).collect()
}

/// `n` points evenly around a vertical circle (starting at +X).
fn ring(center: [f64; 3], diameter: f64, n: u32) -> Vec<[f64; 3]> {
    let r = diameter / 2.0;
    (0..n.clamp(1, 1000))
        .map(|k| {
            let a = f64::from(k) * std::f64::consts::TAU / f64::from(n.max(1));
            [round(center[0] + r * a.cos()), round(center[1] + r * a.sin()), center[2]]
        })
        .collect()
}

/// A face of the program as a measured element, probed as its setup says.
fn face_element(model: &solidcraft_doc::ModelState, settings: &Settings, e: &Entry) -> Result<pg::Element, String> {
    let (info, name) = (&e.info, &e.name);
    let own = settings.of_face(info);
    let (geometry, strategy) = match &info.kind {
        FaceKind::Plane { normal, min, max } => {
            let geometry = pg::Geometry::Plane { normal: normal.vector(), point: *min, min: *min, max: *max };
            let (how, points, picks) = match own.map(|c| &c.kind) {
                Some(CharacteristicKind::Plane { strategy, points, picks, .. }) => (*strategy, *points, picks.as_slice()),
                _ => (settings.plane_default, 4, &[][..]),
            };
            let strategy = match how {
                PlaneProbing::Points if !picks.is_empty() => {
                    let at = check_picks(model, info, picks, MIN_PICKS)?;
                    pg::Strategy::Points { count: at.len() as u32, at }
                }
                PlaneProbing::Points => {
                    let at = spread_points(info, points.clamp(3, 100) as usize)?;
                    if at.len() < 3 {
                        return Err(tr!("there is room for fewer than 3 probing points on the face").into());
                    }
                    pg::Strategy::Points { count: at.len() as u32, at }
                }
                PlaneProbing::Circles => {
                    let want = if normal.axis() == 3 { 4 } else { 2 };
                    let circles = circles(info, want)?;
                    pg::Strategy::CirclesOnPlane {
                        circles: circles.into_iter().map(|(center, diameter)| pg::PathCircle { center, diameter }).collect(),
                    }
                }
            };
            (geometry, strategy)
        }
        FaceKind::Bore { x, y, diameter, top, depth } => {
            let geometry = pg::Geometry::Cylinder { top: [*x, *y, *top], axis: [0.0, 0.0, -1.0], diameter: *diameter, depth: *depth, inner: true };
            let strategy = match own.map(|c| &c.kind) {
                Some(CharacteristicKind::Cylinder { strategy, paths, points, height, picks, .. }) => match strategy {
                    CylinderProbing::Scan => {
                        check_margin(info, *height, *paths)?;
                        pg::Strategy::CylinderScan { paths: (*paths).clamp(1, 20), step: 0.1, margin: *height }
                    }
                    CylinderProbing::CirclePoints => {
                        check_margin(info, *height, *paths)?;
                        pg::Strategy::CylinderPoints { paths: (*paths).clamp(1, 20), points: (*points).clamp(3, 100), margin: *height }
                    }
                    CylinderProbing::Points => {
                        let at = check_picks(model, info, picks, MIN_PICKS)?;
                        pg::Strategy::Points { count: at.len() as u32, at }
                    }
                },
                // a bore of the alignment or a distance: scanned, 2 mm from its ends where it is deep enough
                _ => pg::Strategy::CylinderScan { paths: 2, step: 0.1, margin: round((depth * 0.15).min(2.0)) },
            };
            (geometry, strategy)
        }
    };
    // a plane or cylinder measurement of the face says which probe and safety plane it gets
    let how = own.filter(|c| c.probe_file.is_some() || c.safety_plane.is_some()).map_or_else(|| e.how.clone(), |c| how_of(settings.setup, c));
    Ok(pg::Element { name: name.clone(), geometry, strategy, safety_plane: safety_label(how.safety), probe_file: how.probe })
}

/// The circle a circle measurement probes, as an element.
/// A circle the alignment measures in a bore.
struct AlignCircle {
    x: f64,
    y: f64,
    diameter: f64,
    name: String,
}

/// The circle element for an alignment bore: once per bore; a circle characteristic on the
/// bore gives its name, points, height, safety plane and probe; else 4 points 2 mm below the
/// opening.
fn align_circle(
    model: &solidcraft_doc::ModelState,
    setup: &Setup,
    f: FaceInfo,
    sp: &How,
    circles: &mut Vec<AlignCircle>,
    others: &mut Vec<pg::Element>,
) -> Result<String, String> {
    let FaceKind::Bore { x, y, diameter, .. } = f.kind else { return Err(tr!("choose a bore").into()) };
    let same = |cx: f64, cy: f64, d: f64| (x - cx).hypot(y - cy) < 1e-4 && (diameter - d).abs() < 1e-4;
    if let Some(c) = circles.iter().find(|c| same(c.x, c.y, c.diameter)) {
        return Ok(c.name.clone());
    }
    let own = setup.characteristics.iter().take(MAX_CHARACTERISTICS).find_map(|c| match &c.kind {
        CharacteristicKind::Circle { face, points, height, .. } => match bore_of(model, face).map(|b| b.kind) {
            Ok(FaceKind::Bore { x: cx, y: cy, diameter: d, .. }) if same(cx, cy, d) => Some((c, *points, *height)),
            _ => None,
        },
        _ => None,
    });
    let e = match own {
        Some((c, points, height)) => circle_element(&f, characteristic_name(model, c), points, height, &how_of(setup, c))?,
        None => {
            let name = circle_name(&f);
            circle_element(&f, name, 4, 2.0, sp)?
        }
    };
    circles.push(AlignCircle { x, y, diameter, name: e.name.clone() });
    let name = e.name.clone();
    others.push(e);
    Ok(name)
}

fn circle_element(f: &FaceInfo, name: String, points: u32, height: f64, how: &How) -> Result<pg::Element, String> {
    let FaceKind::Bore { diameter, .. } = f.kind else { return Err(tr!("choose a bore").into()) };
    let center = circle_center(f, height)?;
    let n = points.clamp(3, 100);
    Ok(pg::Element {
        name,
        geometry: pg::Geometry::Circle { center, normal: [0.0, 0.0, 1.0], diameter, inner: true },
        strategy: pg::Strategy::Points { count: n, at: ring(center, diameter, n) },
        safety_plane: safety_label(how.safety),
        probe_file: how.probe.clone(),
    })
}

/// An inclined plane (for an angle), probed with 4 points spread over it.
fn inclined_element(model: &solidcraft_doc::ModelState, r: &FaceRef, name: String, how: &How) -> Result<pg::Element, String> {
    let normal = plane_normal(model, r)?;
    let at = plane_points(model, r, 4)?;
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &at {
        for i in 0..3 {
            if let (Some(l), Some(h), Some(v)) = (lo.get_mut(i), hi.get_mut(i), p.get(i)) {
                *l = l.min(*v);
                *h = h.max(*v);
            }
        }
    }
    Ok(pg::Element {
        name,
        geometry: pg::Geometry::Plane { normal: normal.map(round), point: to_part(r.point).map(round), min: lo, max: hi },
        strategy: pg::Strategy::Points { count: at.len() as u32, at },
        safety_plane: safety_label(how.safety),
        probe_file: how.probe.clone(),
    })
}

/// A point measurement as an element.
fn point_element(model: &solidcraft_doc::ModelState, face: &FaceRef, name: String, how: &How) -> Result<pg::Element, String> {
    let normal = surface_normal(model, face)?;
    let point = to_part(face.point).map(round);
    Ok(pg::Element {
        name,
        geometry: pg::Geometry::Point { point, normal },
        strategy: pg::Strategy::Points { count: 1, at: vec![point] },
        safety_plane: safety_label(how.safety),
        probe_file: how.probe.clone(),
    })
}

/// The safety plane and probe of a characteristic's elements.
fn how_of(setup: &Setup, c: &Characteristic) -> How {
    How { safety: c.safety_plane.unwrap_or(setup.safety_plane), probe: c.probe_file.clone().filter(|f| setup.probe.probe_file.as_ref() != Some(f)) }
}

/// The elements a characteristic probes (for showing its points and paths on the model while
/// its dialog is open or it is selected), probed as `setup` says.
pub fn characteristic_elements(model: &solidcraft_doc::ModelState, setup: &Setup, c: &Characteristic) -> Result<Vec<pg::Element>, String> {
    in_alignment(model, &setup.alignment, || {
        let mut els = characteristic_elements_part(model, setup, c)?;
        els.iter_mut().for_each(element_to_model);
        Ok(els)
    })
}

/// [`characteristic_elements`] in part axes.
fn characteristic_elements_part(model: &solidcraft_doc::ModelState, setup: &Setup, c: &Characteristic) -> Result<Vec<pg::Element>, String> {
    let settings = Settings::of(model, setup, Some(c));
    let sp = how_of(setup, c);
    let entry = |r: &FaceRef| -> Result<pg::Element, String> {
        let info = analyze(model, r)?;
        let name = default_name(&info);
        face_element(model, &settings, &Entry { info, name, how: sp.clone() })
    };
    Ok(match &c.kind {
        CharacteristicKind::Circle { face, points, height, .. } => {
            vec![circle_element(&bore_of(model, face)?, tr!("Circle").into(), *points, *height, &sp)?]
        }
        CharacteristicKind::Cylinder { face, .. } | CharacteristicKind::Plane { face, .. } => vec![entry(face)?],
        CharacteristicKind::Distance { from, to, .. } => vec![entry(from)?, entry(to)?],
        CharacteristicKind::Angle { first, second } => {
            let mut v = Vec::new();
            for r in [first, second] {
                v.push(match analyze(model, r) {
                    Ok(f) if matches!(f.kind, FaceKind::Plane { .. }) => entry(r)?,
                    _ => inclined_element(model, r, tr!("Plane").into(), &sp)?,
                });
            }
            v
        }
        CharacteristicKind::Point { points, .. } => points
            .iter()
            .take(MAX_POINTS)
            .enumerate()
            .map(|(i, f)| point_element(model, f, tr!("Point {n}", n = i + 1), &sp))
            .collect::<Result<_, _>>()?,
    })
}

/// What an element's probing looks like on the model: `point` {at, normal} and `circle`
/// {center, normal, radius, start_normal} markers (normals point out of the material; a
/// circle's start is at its local +X: +X for a horizontal circle, `start_normal` the surface
/// normal there).
pub fn element_markers(e: &pg::Element) -> Vec<Value> {
    let inward_of = |axis_point: [f64; 2], p: [f64; 3]| {
        let (dx, dy) = (axis_point[0] - p[0], axis_point[1] - p[1]);
        let l = (dx * dx + dy * dy).sqrt();
        if l > 1e-9 { [dx / l, dy / l, 0.0] } else { [1.0, 0.0, 0.0] }
    };
    let mut out = Vec::new();
    match (&e.geometry, &e.strategy) {
        (pg::Geometry::Plane { normal, .. }, pg::Strategy::CirclesOnPlane { circles }) => {
            for c in circles.iter().take(200) {
                out.push(json!({"type": "circle", "center": c.center, "normal": normal, "radius": c.diameter / 2.0, "start_normal": normal}));
            }
        }
        (pg::Geometry::Cylinder { top, diameter, depth, .. }, pg::Strategy::CylinderScan { paths, margin, .. }) => {
            for z in path_heights(top[2], *depth, *margin, *paths) {
                // the path starts at +X, where the wall looks to the axis (−X)
                out.push(json!({"type": "circle", "center": [top[0], top[1], z], "normal": [0.0, 0.0, 1.0], "radius": diameter / 2.0, "start_normal": [-1.0, 0.0, 0.0]}));
            }
        }
        (pg::Geometry::Cylinder { top, diameter, depth, .. }, pg::Strategy::CylinderPoints { paths, points, margin }) => {
            for z in path_heights(top[2], *depth, *margin, *paths) {
                for p in ring([top[0], top[1], z], *diameter, *points) {
                    out.push(json!({"type": "point", "at": p, "normal": inward_of([top[0], top[1]], p)}));
                }
            }
        }
        (geometry, pg::Strategy::Points { at, .. }) => {
            for p in at.iter().take(MAX_PICKS * 2) {
                let n = match geometry {
                    pg::Geometry::Plane { normal, .. } | pg::Geometry::Point { normal, .. } => *normal,
                    pg::Geometry::Cylinder { top, .. } => inward_of([top[0], top[1]], *p),
                    pg::Geometry::Circle { center, .. } => inward_of([center[0], center[1]], *p),
                };
                out.push(json!({"type": "point", "at": p, "normal": n}));
            }
        }
        _ => {}
    }
    out
}

/// The markers of a characteristic (see [`element_markers`]).
pub fn characteristic_markers(model: &solidcraft_doc::ModelState, setup: &Setup, c: &Characteristic) -> Value {
    in_alignment(model, &setup.alignment, || match characteristic_elements_part(model, setup, c) {
        Ok(els) => {
            let mut markers: Vec<Value> = els.iter().flat_map(element_markers).collect();
            markers_to_model(&mut markers);
            json!(markers)
        }
        Err(_) => json!([]),
    })
}

/// The clamping plate as the primary plane (`Alignment::clamp`), in part axes: a plane through
/// the primary face looking up (+Z), probed at the given points or `margin` outside the corners
/// of the part's footprint. `frame`: the alignment's coordinate system in part axes.
fn clamp_element(model: &solidcraft_doc::ModelState, setup: &Setup, clamp: &ClampPlane, frame: &Frame, name: String) -> Result<pg::Element, String> {
    let z0 = frame.origin[2];
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for name in setup_bodies(setup) {
        let Some(b) = model.body(&name) else { continue };
        let m = b.mesh();
        for p in m.positions.iter().take(500_000) {
            let q = to_part(arr(*p));
            for i in 0..2 {
                lo[i] = lo[i].min(q[i]);
                hi[i] = hi[i].max(q[i]);
            }
        }
    }
    if !(lo[0].is_finite() && hi[0].is_finite() && lo[1].is_finite() && hi[1].is_finite()) {
        return Err(tr!("the setup measures no body yet").into());
    }
    let at: Vec<[f64; 3]> = if clamp.points.is_empty() {
        let m = if clamp.margin.is_finite() && clamp.margin > 0.0 { clamp.margin.min(1000.0) } else { 10.0 };
        vec![[lo[0] - m, lo[1] - m, z0], [hi[0] + m, lo[1] - m, z0], [hi[0] + m, hi[1] + m, z0], [lo[0] - m, hi[1] + m, z0]]
    } else {
        // X and Y in the alignment's coordinate system
        let mut v = Vec::new();
        for (i, q) in clamp.points.iter().take(MAX_CLAMP_POINTS).enumerate() {
            if !(q[0].is_finite() && q[1].is_finite() && q[0].abs() < 1e5 && q[1].abs() < 1e5) {
                return Err(tr!("clamping plate: point {i} is not valid", i = i + 1));
            }
            let p = [0, 1, 2].map(|k| frame.origin[k] + frame.x[k] * q[0] + frame.y[k] * q[1]);
            if p[0] > lo[0] && p[0] < hi[0] && p[1] > lo[1] && p[1] < hi[1] {
                return Err(tr!("clamping plate: point {i} lies under the part", i = i + 1));
            }
            v.push([p[0], p[1], z0]);
        }
        if v.len() < 3 {
            return Err(tr!("clamping plate: give at least 3 points (or none for the automatic ones)").into());
        }
        v
    };
    let at: Vec<[f64; 3]> = at.into_iter().map(|p| p.map(round)).collect();
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &at {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    Ok(pg::Element {
        name,
        geometry: pg::Geometry::Plane { normal: [0.0, 0.0, 1.0], point: at.first().copied().unwrap_or([0.0, 0.0, z0]), min, max },
        strategy: pg::Strategy::Points { count: at.len() as u32, at },
        safety_plane: Some("+Z".into()),
        probe_file: None,
    })
}

fn expect_plane(info: &FaceInfo, role: &str) -> Result<(), String> {
    match info.kind {
        FaceKind::Plane { .. } => Ok(()),
        FaceKind::Bore { .. } => Err(tr!("{role}: choose a plane, not a bore", role = role)),
    }
}

fn expect_bore(info: &FaceInfo, role: &str) -> Result<(), String> {
    match info.kind {
        FaceKind::Bore { .. } => Ok(()),
        FaceKind::Plane { .. } => Err(tr!("{role}: choose a bore, not a plane", role = role)),
    }
}

/// The outward normal of the face at a point (the nearest mesh triangle's).
/// How far a point may lie from the part's surface and still count as on it (mm; the mesh of a
/// curved face stands off its true surface by up to its tolerance).
const ON_SURFACE: f64 = 0.5;

/// A point picked on the part must lie on its surface.
fn on_surface(model: &solidcraft_doc::ModelState, r: &FaceRef) -> Result<(), String> {
    let b = model.body(&r.body).ok_or_else(|| tr!("Body {body} does not exist (any more)", body = format!("{:?}", r.body)))?;
    let m = b.mesh();
    let p = v3(r.point);
    let d2 = m.triangles.iter().filter_map(|t| m.tri(t)).map(|tri| dist2_triangle(p, &tri)).fold(f64::INFINITY, f64::min);
    if d2.is_finite() && d2 <= ON_SURFACE * ON_SURFACE {
        Ok(())
    } else {
        Err(tr!("the point X{x} Y{y} Z{z} is not on the body", x = num(r.point[0]), y = num(r.point[1]), z = num(r.point[2])))
    }
}

fn surface_normal(model: &solidcraft_doc::ModelState, r: &FaceRef) -> Result<[f64; 3], String> {
    on_surface(model, r)?;
    if let Ok(n) = plane_normal(model, r) {
        return Ok(n);
    }
    // a bore: exactly radial, out of the material (towards the axis)
    if let Ok(FaceInfo { kind: FaceKind::Bore { x, y, .. }, .. }) = analyze(model, r) {
        let q = to_part(r.point);
        let d = [x - q[0], y - q[1], 0.0];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if l > 1e-9 {
            return Ok([d[0] / l, d[1] / l, 0.0].map(round));
        }
    }
    let b = model.body(&r.body).ok_or_else(|| tr!("Body {body} does not exist (any more)", body = format!("{:?}", r.body)))?;
    let m = b.mesh();
    let p = v3(r.point);
    let (_, t, tri) = m
        .triangles
        .iter()
        .filter_map(|t| m.tri(t).map(|tri| (dist2_triangle(p, &tri), t, tri)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .ok_or_else(|| tr!("Body {body} has no faces", body = format!("{:?}", r.body)))?;
    let flat = arr(tri_normal(tri));
    // any other curved face: the surface normals at the triangle's corners, interpolated at the
    // point (the facet's own normal is off by the facet angle), on the facet's side
    Ok(to_part(smooth_normal(&m, t, &tri, r.point).filter(|n| dot3(*n, flat) > 0.5).unwrap_or(flat)).map(round))
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The mesh's vertex normals of triangle `t`, interpolated at `p` (barycentric, clamped to the
/// triangle) and normalised; `None` without usable normals.
fn smooth_normal(m: &solidcraft_geom::Mesh, t: &[u32; 3], tri: &[Vec3; 3], p: [f64; 3]) -> Option<[f64; 3]> {
    let ns: Vec<[f64; 3]> = t.iter().filter_map(|i| m.normals.get(*i as usize)).map(|n| arr(*n)).collect();
    let [n0, n1, n2] = <[[f64; 3]; 3]>::try_from(ns).ok()?;
    let [a, b, c] = tri.map(arr);
    let sub = |u: [f64; 3], v: [f64; 3]| [u[0] - v[0], u[1] - v[1], u[2] - v[2]];
    let (v0, v1, v2) = (sub(b, a), sub(c, a), sub(p, a));
    let (d00, d01, d11, d20, d21) = (dot3(v0, v0), dot3(v0, v1), dot3(v1, v1), dot3(v2, v0), dot3(v2, v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-18 {
        return None;
    }
    let v = ((d11 * d20 - d01 * d21) / den).clamp(0.0, 1.0);
    let w = ((d00 * d21 - d01 * d20) / den).clamp(0.0, 1.0 - v);
    let u = 1.0 - v - w;
    let n = [0, 1, 2].map(|i| u * n0[i] + v * n1[i] + w * n2[i]);
    let l = dot3(n, n).sqrt();
    (l.is_finite() && l > 1e-9).then(|| n.map(|x| x / l))
}

/// Up to `count` points spread over a plane face (any orientation), at least `margin` from its
/// edges where the face allows, for probing it with single points.
fn plane_points(model: &solidcraft_doc::ModelState, r: &FaceRef, count: usize) -> Result<Vec<[f64; 3]>, String> {
    let b = model.body(&r.body).ok_or_else(|| tr!("Body {body} does not exist (any more)", body = format!("{:?}", r.body)))?;
    let m = b.mesh();
    let p = v3(r.point);
    let (_, face) = m
        .triangles
        .iter()
        .zip(&m.tri_face)
        .filter_map(|(t, f)| m.tri(t).map(|tri| (dist2_triangle(p, &tri), *f as usize)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .ok_or_else(|| tr!("Body {body} has no faces", body = format!("{:?}", r.body)))?;
    let patch = m.surface_patch(face);
    // candidates inside each triangle: its centroid and a point towards each corner, kept
    // in from the edges (a rectangle is only two triangles)
    let at = |t: &[Vec3; 3], w: [f64; 3]| Vec3 {
        x: t[0].x * w[0] + t[1].x * w[1] + t[2].x * w[2],
        y: t[0].y * w[0] + t[1].y * w[1] + t[2].y * w[2],
        z: t[0].z * w[0] + t[1].z * w[1] + t[2].z * w[2],
    };
    let cands: Vec<Vec3> = m
        .triangles
        .iter()
        .zip(&m.tri_face)
        .filter(|(_, f)| patch.contains(&(**f as usize)))
        .filter_map(|(t, _)| m.tri(t))
        .take(5000)
        .flat_map(|t| [[1.0 / 3.0; 3], [0.7, 0.15, 0.15], [0.15, 0.7, 0.15], [0.15, 0.15, 0.7]].map(|w| at(&t, w)))
        .collect();
    let mut chosen: Vec<Vec3> = Vec::new();
    while chosen.len() < count.min(cands.len()) {
        let far = |c: &Vec3| chosen.iter().map(|x| dotv(sub(*c, *x), sub(*c, *x))).fold(f64::INFINITY, f64::min);
        let next = if chosen.is_empty() { cands.first().copied() } else { cands.iter().copied().max_by(|a, b| far(a).total_cmp(&far(b))) };
        match next {
            Some(c) if chosen.is_empty() || far(&c) > 1e-6 => chosen.push(c),
            _ => break,
        }
    }
    if chosen.is_empty() {
        return Err(tr!("the face has no triangles").into());
    }
    Ok(chosen.into_iter().map(|c| to_part(arr(c)).map(round)).collect())
}

/// A setup as the neutral CMM program its post translates.
pub fn program(model: &solidcraft_doc::ModelState, setup: &Setup, nc: &NcProgram, plan: &str) -> Result<pg::Program, String> {
    in_alignment(model, &setup.alignment, || program_part(model, setup, nc, plan))
}

/// [`program`], worked out in part axes and handed out in model coordinates.
fn program_part(model: &solidcraft_doc::ModelState, setup: &Setup, nc: &NcProgram, plan: &str) -> Result<pg::Program, String> {
    let a = &setup.alignment;
    // the whole alignment must hold before anything is written
    let fr = frame_part(model, a)?;
    let settings = Settings::of(model, setup, None);
    let sp = How { safety: setup.safety_plane, probe: None };
    let mut reg = Registry { faces: Vec::new() };
    let mut others: Vec<pg::Element> = Vec::new();
    let an = |r: &FaceRef| analyze(model, r);
    let primary = an(need(&a.primary, tr!("the primary plane"))?)?;
    expect_plane(&primary, tr!("Primary plane"))?;
    let primary_name = match &a.clamp {
        // the clamping plate beside the part, not the face itself
        Some(clamp) => {
            let name = reg.unique(tr!("Clamping plate"), &others);
            others.push(clamp_element(model, setup, clamp, &fr, name.clone())?);
            name
        }
        None => reg.add(primary, None, &sp),
    };
    // the alignment's bores: cylinders in the registry, or circles of their own
    let mut circles: Vec<AlignCircle> = Vec::new();
    let mut bore_element = |f: FaceInfo, reg: &mut Registry, others: &mut Vec<pg::Element>| -> Result<String, String> {
        if a.bores == AlignBores::Cylinder {
            return Ok(reg.add(f, None, &sp));
        }
        align_circle(model, setup, f, &sp, &mut circles, others)
    };
    let direction = match need(&a.direction, tr!("the direction"))? {
        DirectionRef::Side { face } => {
            let f = an(face)?;
            expect_plane(&f, tr!("Direction"))?;
            pg::Direction::Side { plane: reg.add(f, None, &sp) }
        }
        DirectionRef::Bores { first, second } => {
            let (p, q) = (an(first)?, an(second)?);
            expect_bore(&p, tr!("Direction"))?;
            expect_bore(&q, tr!("Direction"))?;
            let from = bore_element(p, &mut reg, &mut others)?;
            pg::Direction::Line { from, to: bore_element(q, &mut reg, &mut others)? }
        }
    };
    let mut origin = |o: &OriginRef, reg: &mut Registry, others: &mut Vec<pg::Element>| -> Result<pg::Origin, String> {
        Ok(match o {
            OriginRef::Side { face } => {
                let f = an(face)?;
                expect_plane(&f, tr!("Origin"))?;
                pg::Origin::Side { plane: reg.add(f, None, &sp) }
            }
            OriginRef::Middle { first, second } => {
                let (p, q) = (an(first)?, an(second)?);
                expect_plane(&p, tr!("Origin (middle)"))?;
                expect_plane(&q, tr!("Origin (middle)"))?;
                pg::Origin::Middle { first: reg.add(p, None, &sp), second: reg.add(q, None, &sp) }
            }
            OriginRef::Bore { face } => {
                let f = an(face)?;
                expect_bore(&f, tr!("Origin"))?;
                pg::Origin::Axis { element: bore_element(f, reg, others)? }
            }
        })
    };
    let origin_x = origin(need(&a.origin_x, tr!("the origin X"))?, &mut reg, &mut others)?;
    let origin_y = origin(need(&a.origin_y, tr!("the origin Y"))?, &mut reg, &mut others)?;
    // the circle characteristics the alignment measures already
    let aligned: Vec<String> = circles.iter().map(|c| c.name.clone()).collect();
    let pg_axis = |x: Axis| match x {
        Axis::X => pg::Axis::X,
        Axis::Y => pg::Axis::Y,
        Axis::Z => pg::Axis::Z,
    };
    let mut chars: Vec<pg::Characteristic> = Vec::new();
    for c in setup.characteristics.iter().take(MAX_CHARACTERISTICS) {
        let name = characteristic_name(model, c);
        let csp = how_of(setup, c);
        let tol = pg::Tolerance::symmetric(if c.tol.is_finite() && c.tol >= 0.0 { c.tol } else { 0.05 });
        let ch = |kind: pg::CharacteristicKind| pg::Characteristic { name: name.clone(), kind, tol };
        let coords = |element: &str, p: [f64; 3], axes: &[Axis]| -> Vec<pg::Characteristic> {
            let l = fr.local(p);
            axes.iter()
                .map(|x| pg::Characteristic {
                    name: format!("{name} {}", x.letter()),
                    kind: pg::CharacteristicKind::Coordinate {
                        element: element.to_string(),
                        axis: pg_axis(*x),
                        nominal: round(l.get(*x as usize).copied().unwrap_or(0.0)),
                    },
                    tol,
                })
                .collect()
        };
        let named = |e: String| format!("{name}: {e}");
        match &c.kind {
            CharacteristicKind::Cylinder { face, .. } => {
                let f = bore_of(model, face).map_err(named)?;
                let FaceKind::Bore { diameter, .. } = f.kind else { continue };
                let element = reg.add(f, Some(&name), &csp);
                chars.push(ch(pg::CharacteristicKind::Diameter { element, nominal: diameter }));
            }
            CharacteristicKind::Plane { face, .. } => {
                let f = plane_of(model, face).map_err(named)?;
                reg.add(f, Some(&name), &csp);
            }
            CharacteristicKind::Circle { face, points, height, position } => {
                let f = bore_of(model, face).map_err(named)?;
                // a circle the alignment measures is that element already
                let (element, e) = if aligned.contains(&name) {
                    let e = others.iter().find(|e| e.name == name).cloned().ok_or_else(|| named(tr!("the alignment's circle is missing").into()))?;
                    (name.clone(), e)
                } else {
                    let element = reg.unique(&name, &others);
                    let e = circle_element(&f, element.clone(), *points, *height, &csp).map_err(named)?;
                    others.push(e.clone());
                    (element, e)
                };
                let (center, diameter) = match &e.geometry {
                    pg::Geometry::Circle { center, diameter, .. } => (*center, *diameter),
                    _ => continue,
                };
                chars.push(ch(pg::CharacteristicKind::Diameter { element: element.clone(), nominal: diameter }));
                if *position {
                    chars.extend(coords(&element, center, &[Axis::X, Axis::Y]));
                }
            }
            CharacteristicKind::Distance { from, to, axis } => {
                let (p, q) = (an(from).map_err(named)?, an(to).map_err(named)?);
                let (ax, nominal) = distance(&p, &q, *axis).map_err(named)?;
                let (from, to) = (reg.add(p, None, &csp), reg.add(q, None, &csp));
                chars.push(ch(pg::CharacteristicKind::Distance { from, to, axis: pg_axis(ax), nominal }));
            }
            CharacteristicKind::Angle { first, second } => {
                let nominal = angle(model, first, second).map_err(named)?;
                let mut names = Vec::new();
                for r in [first, second] {
                    // an axis-parallel plane is probed like the others, an inclined one with points
                    match an(r) {
                        Ok(f) if matches!(f.kind, FaceKind::Plane { .. }) => names.push(reg.add(f, None, &csp)),
                        _ => {
                            let n = reg.unique(&tr!("Plane inclined {n}", n = others.len() + 1), &others);
                            others.push(inclined_element(model, r, n.clone(), &csp).map_err(named)?);
                            names.push(n);
                        }
                    }
                }
                let (first, second) = (names.first().cloned().unwrap_or_default(), names.get(1).cloned().unwrap_or_default());
                chars.push(ch(pg::CharacteristicKind::Angle { first, second, nominal }));
            }
            CharacteristicKind::Point { points, check } => {
                let many = points.len() > 1;
                for (i, face) in points.iter().take(MAX_POINTS).enumerate() {
                    let label = if many { format!("{name} P{}", i + 1) } else { name.clone() };
                    let element = reg.unique(&label, &others);
                    let e = point_element(model, face, element.clone(), &csp).map_err(named)?;
                    others.push(e);
                    match check {
                        PointCheck::Normal => {
                            chars.push(pg::Characteristic { name: label, kind: pg::CharacteristicKind::Deviation { element }, tol })
                        }
                        PointCheck::Coordinates => {
                            let l = fr.local(to_part(face.point).map(round));
                            for x in [Axis::X, Axis::Y, Axis::Z] {
                                chars.push(pg::Characteristic {
                                    name: format!("{label} {}", x.letter()),
                                    kind: pg::CharacteristicKind::Coordinate {
                                        element: element.clone(),
                                        axis: pg_axis(x),
                                        nominal: round(l.get(x as usize).copied().unwrap_or(0.0)),
                                    },
                                    tol,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    // bounding box of the bodies used
    let mut bbox = [f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY];
    let mut bodies: Vec<&str> = reg.faces.iter().map(|e| e.info.body.as_str()).collect();
    bodies.sort_unstable();
    bodies.dedup();
    for name in bodies {
        if let Some(b) = model.body(name) {
            // the box in model coordinates around the part as it lies (turned or not)
            for p in b.mesh().positions.iter().take(500_000) {
                for (i, c) in arr(*p).into_iter().enumerate() {
                    if let (Some(a), Some(z)) = (bbox.get(2 * i).copied(), bbox.get(2 * i + 1).copied()) {
                        bbox[2 * i] = a.min(c);
                        bbox[2 * i + 1] = z.max(c);
                    }
                }
            }
        }
    }
    let bbox = bbox.map(round);
    let mut elements = Vec::new();
    for e in &reg.faces {
        elements.push(face_element(model, &settings, e).map_err(|x| format!("{}: {x}", e.name))?);
    }
    elements.extend(others);
    // the clamping plate is measured first, as the primary plane
    if let Some(i) = elements.iter().position(|e| e.name == primary_name) {
        let first = elements.remove(i);
        elements.insert(0, first);
    }
    // worked out in part axes, handed out in model coordinates
    elements.iter_mut().for_each(element_to_model);
    let fr = fr.to_model();
    let mut named: Vec<&str> = setup.probe.probe_file.as_deref().into_iter().collect();
    named.extend(elements.iter().filter_map(|e| e.probe_file.as_deref()));
    let probe_balls = probe_balls(&named);
    let p = pg::Program {
        format: pg::FORMAT.into(),
        plan: plan.to_string(),
        setup: setup.name.clone(),
        probe_file: setup.probe.probe_file.clone(),
        post_options: nc.options.clone(),
        elements,
        alignment: pg::Alignment { primary: primary_name, direction, origin_x, origin_y },
        characteristics: chars,
        safety: pg::Safety { min: [bbox[0], bbox[2], bbox[4]], max: [bbox[1], bbox[3], bbox[5]], clearance: 10.0 },
        base_system: Some(pg::BaseSystem { origin: fr.origin, x: fr.x, y: fr.y, z: fr.z }),
        probe_balls,
    };
    p.validate()?;
    Ok(p)
}

// ------------------------------------------------------------------------------------------
// The state the tree and the dialogs show
// ------------------------------------------------------------------------------------------

/// One setup, described.
pub fn setup_info(model: &solidcraft_doc::ModelState, plans: &CmmPlans, setup: &Setup, design_path: Option<&str>) -> Value {
    in_alignment(model, &setup.alignment, || setup_info_part(model, plans, setup, design_path))
}

fn setup_info_part(model: &solidcraft_doc::ModelState, plans: &CmmPlans, setup: &Setup, design_path: Option<&str>) -> Value {
    let fr = frame(model, &setup.alignment);
    let frame_ok = fr.as_ref().ok();
    let characteristics: Vec<Value> = setup
        .characteristics
        .iter()
        .map(|c| {
            let mut v = characteristic_info(model, frame_ok, c);
            v["markers"] = characteristic_markers(model, setup, c);
            v
        })
        .collect();
    let nc = default_program(setup);
    let name = plan_name(setup, &nc, design_path);
    let problem = match &fr {
        Err(e) => Some(e.clone()),
        Ok(_) => characteristics
            .iter()
            .find_map(|c| c["error"].as_str().map(|e| format!("{}: {e}", c["name"].as_str().unwrap_or(""))))
            // what only the whole program finds (a face's strategy that does not fit, …)
            .or_else(|| program(model, &for_program(model, setup, &nc), &nc, &name).err()),
    };
    json!({
        "id": setup.id,
        "name": setup.name,
        "active": plans.active_setup().is_some_and(|a| a.id == setup.id),
        "probe": {
            "probe_file": setup.probe.probe_file,
            "probe_geometry": setup.probe.probe_file.as_deref().and_then(probe_row),
        },
        "safety_plane": setup.safety_plane,
        "plane_probing": setup.plane_probing,
        "alignment": {
            "set": alignment_text(model, &setup.alignment),
            "complete": fr.is_ok(),
            "bores": setup.alignment.bores,
            "elements": alignment_elements(model, setup),
            "problem": fr.as_ref().err(),
            "frame": frame_ok.map(Frame::to_json),
        },
        "characteristics": characteristics,
        "programs": setup.programs.iter().map(|nc| program_info(setup, nc, design_path)).collect::<Vec<_>>(),
        "ready": problem.is_none(),
        "problem": problem,
    })
}

// ------------------------------------------------------------------------------------------
// Probe table
// ------------------------------------------------------------------------------------------

/// The format tag of the probe table file.
pub const PROBE_TABLE_FORMAT: &str = "solidcraft-probe-table/1";
/// Most rows the probe table holds.
pub const MAX_PROBE_ROWS: usize = 500;

/// One probe (stylus) of the probe table: its dimensions in mm, for showing it on the model and
/// later for checking approach paths. `file` is the probe configuration's name: its file name
/// without the extension.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProbeRow {
    pub file: String,
    /// Ø of the ball.
    pub ball: Option<f64>,
    /// Length from the ball centre to the probe's mount.
    pub length: Option<f64>,
    /// Measuring length: ball centre to the end of the thin shaft.
    pub measuring_length: Option<f64>,
    /// Ø of the shaft.
    pub shaft: Option<f64>,
    /// Second measuring length, for a stepped shaft.
    pub measuring_length2: Option<f64>,
    /// Ø of the second shaft.
    pub shaft2: Option<f64>,
    /// Ø of the base body.
    pub body: Option<f64>,
    pub order_no: String,
    pub note: String,
}

impl ProbeRow {
    /// Checks the numbers (positive, finite, below 2 m).
    pub fn validate(&self) -> Result<(), String> {
        if self.file.trim().is_empty() || self.file.len() > 200 || self.order_no.len() > 200 || self.note.len() > 1000 {
            return Err(tr!("Probe: the name (probe file) is missing or a text is too long").into());
        }
        for (label, v) in [
            (tr!("ball Ø"), self.ball),
            (tr!("length"), self.length),
            (tr!("measuring length"), self.measuring_length),
            (tr!("shaft Ø"), self.shaft),
            (tr!("2nd measuring length"), self.measuring_length2),
            (tr!("2nd shaft Ø"), self.shaft2),
            (tr!("base body Ø"), self.body),
        ] {
            if let Some(x) = v
                && !(x.is_finite() && x > 0.0 && x < 2000.0)
            {
                return Err(tr!("{probe}: {what} must be a number above 0", probe = self.file, what = label));
            }
        }
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ProbeTableFile {
    format: String,
    probes: Vec<ProbeRow>,
}

/// `<data>/inspection/probes/probe-table.json` (local only, like the probe files; the first
/// versions' name is still read while only that one exists).
pub fn probe_table_path() -> Option<PathBuf> {
    let dir = probe_folder()?;
    let (new, old) = (dir.join("probe-table.json"), dir.join("tastertabelle.json"));
    Some(if !new.exists() && old.is_file() { old } else { new })
}

/// The probes a setup, measurement or characteristic may choose: only the probe table's rows
/// (as the path of its probe file when the probe folder has one, else as its bare name — the
/// post then needs only the configuration name). Nothing else is offered: no probe file without
/// a row, no built-in probes.
pub fn probe_choices() -> Vec<String> {
    choices_of(&saved_probe_table(), probe_files())
}

/// [`probe_choices`] from a table and the probe files: one per table row, as the path of its
/// probe file when the folder has one, else by name. A probe file without a row is not offered.
pub fn choices_of(table: &[ProbeRow], files: Vec<PathBuf>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in table {
        let key = r.file.trim();
        if key.is_empty() || out.iter().any(|o| probe_key(o).eq_ignore_ascii_case(key)) {
            continue;
        }
        match files.iter().find(|f| probe_key(&f.display().to_string()).eq_ignore_ascii_case(key)) {
            Some(f) => out.push(f.display().to_string()),
            None => out.push(key.to_string()),
        }
    }
    out.sort_by_key(|o| probe_key(o).to_ascii_lowercase());
    out.truncate(MAX_PROBE_ROWS);
    out
}

/// Is `f` something a probe choice may name: an existing probe file, or the bare name of a
/// probe table row?
pub fn probe_choice_ok(f: &str) -> bool {
    if Path::new(f).is_file() {
        return true;
    }
    let bare = !f.contains(['/', '\\']);
    bare && saved_probe_table().iter().any(|r| r.file.trim().eq_ignore_ascii_case(&probe_key(f)))
}

/// The ball diameters (probe table) of the probes a program names, by its `probe_file` text.
pub fn probe_balls(names: &[&str]) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for n in names {
        if let Some(b) = probe_row(n).and_then(|r| r.ball).filter(|b| b.is_finite() && *b > 0.0) {
            out.insert((*n).to_string(), b);
        }
    }
    out
}

/// A probe file's table name: its file name without the extension (one of 1–4 letters; a bare
/// probe name stays as it is).
pub fn probe_key(probe_file: &str) -> String {
    // a Windows path is a Windows path on any system
    let f = probe_file.rsplit(['/', '\\']).next().unwrap_or(probe_file);
    match f.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && (1..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphabetic()) => stem.to_string(),
        _ => f.to_string(),
    }
}

/// The saved table (empty when there is none or it cannot be read).
pub fn saved_probe_table() -> Vec<ProbeRow> {
    probe_table_path().map(|p| read_probe_table(&p)).unwrap_or_default()
}

/// A probe table file (empty when it cannot be read).
pub fn read_probe_table(p: &Path) -> Vec<ProbeRow> {
    let Ok(text) = std::fs::read_to_string(p) else { return Vec::new() };
    serde_json::from_str::<ProbeTableFile>(&text)
        .map(|t| t.probes.into_iter().filter(|r| r.validate().is_ok()).take(MAX_PROBE_ROWS).collect())
        .unwrap_or_default()
}

/// Writes a probe table file.
pub fn write_probe_table(p: &Path, rows: &[ProbeRow]) -> Result<(), String> {
    if rows.len() > MAX_PROBE_ROWS {
        return Err(tr!("at most {n} probes", n = MAX_PROBE_ROWS));
    }
    for r in rows {
        r.validate()?;
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let text =
        serde_json::to_string_pretty(&ProbeTableFile { format: PROBE_TABLE_FORMAT.into(), probes: rows.to_vec() }).map_err(|e| e.to_string())?;
    std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))
}

/// The table as the dialog shows it: only the saved rows (new ones are added in the dialog).
pub fn probe_table() -> Vec<ProbeRow> {
    saved_probe_table()
}

/// Saves the table.
pub fn save_probe_table(rows: &[ProbeRow]) -> Result<PathBuf, String> {
    let p = probe_table_path().ok_or(tr!("no data folder"))?;
    write_probe_table(&p, rows)?;
    Ok(p)
}

/// The table row of a probe file.
pub fn probe_row(probe_file: &str) -> Option<ProbeRow> {
    let key = probe_key(probe_file);
    saved_probe_table().into_iter().find(|r| r.file.eq_ignore_ascii_case(&key))
}

/// All CMM plans, described.
pub fn summary(model: &solidcraft_doc::ModelState, plans: &CmmPlans, design_path: Option<&str>) -> Value {
    json!({
        "setups": plans.setups.iter().map(|s| setup_info(model, plans, s, design_path)).collect::<Vec<_>>(),
        "active": plans.active_setup().map(|s| s.id),
        "posts": posts().iter().map(|p| post_name(p)).collect::<Vec<_>>(),
        "probe_files": probe_choices(),
        "data_folder": data_folder().map(|p| p.display().to_string()),
    })
}

/// The bodies a setup measures (by name, sorted): those of its alignment and its
/// characteristics.
pub fn setup_bodies(setup: &Setup) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>, depth: usize) {
        match v {
            Value::Object(m) if depth < 8 => {
                if let (Some(Value::String(b)), Some(Value::Array(_))) = (m.get("body"), m.get("point")) {
                    out.push(b.clone());
                }
                m.values().for_each(|x| walk(x, out, depth + 1));
            }
            Value::Array(a) if depth < 8 => a.iter().for_each(|x| walk(x, out, depth + 1)),
            _ => {}
        }
    }
    let mut out: Vec<String> = setup.characteristics.iter().flat_map(|c| c.kind.faces()).map(|f| f.body.clone()).collect();
    if let Ok(v) = serde_json::to_value(&setup.alignment) {
        walk(&v, &mut out, 0);
    }
    out.sort();
    out.dedup();
    out
}

/// The setup's bodies as a STEP file in the plan's base system (the alignment's coordinate
/// system), so the measuring software places the CAD model on its alignment without moving it.
/// Without a base system the model's own coordinates are kept.
pub fn base_system_step(model: &solidcraft_doc::ModelState, setup: &Setup, program: &pg::Program) -> Result<Vec<u8>, String> {
    let names = setup_bodies(setup);
    if names.is_empty() {
        return Err(tr!("the setup measures no body yet").into());
    }
    // column-major (a column per axis, the last the translation): p' = R (p - o)
    let m = match &program.base_system {
        Some(b) if b.is_valid() => {
            let t = b.direction([-b.origin[0], -b.origin[1], -b.origin[2]]);
            let col = |c: usize| [b.x[c], b.y[c], b.z[c], 0.0];
            [col(0), col(1), col(2), [t[0], t[1], t[2], 1.0]]
        }
        _ => solidcraft_doc::IDENTITY,
    };
    let mut state = solidcraft_doc::ModelState::default();
    for n in &names {
        let b = model.body(n).ok_or_else(|| tr!("Body “{body}” is missing in the model", body = n))?;
        let moved = solidcraft_kernel::transform_matrix(&b.body, m).map_err(|e| format!("{n}: {e}"))?;
        state.bodies.push(solidcraft_doc::ModelBody::new(n.clone(), moved, b.feature));
    }
    solidcraft_io::export_with_metadata(&state, &names, solidcraft_io::Format::Step, &program.plan, &solidcraft_doc::Metadata::default())
        .map_err(|e| e.to_string())
}
