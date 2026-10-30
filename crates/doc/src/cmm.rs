//! Coordinate measuring (CMM) plans, saved with the design: a list of setups, each with its
//! probe system (postprocessor and probe configuration), its alignment (which faces make the part's
//! coordinate system) and its inspection plan (the characteristics to measure).
//!
//! Faces are kept as a body and a point on the face, so they follow rebuilds; the engine reads
//! their geometry from the evaluated model whenever it needs it (`solidcraft_engine::inspection`).
//! Nothing here changes the model: a change to the plans needs no evaluation.

use serde::{Deserialize, Serialize};

/// Most setups a design holds.
pub const MAX_SETUPS: usize = 100;
/// Most characteristics a setup holds.
pub const MAX_CHARACTERISTICS: usize = 500;
/// At most this many NC programs in a setup.
pub const MAX_NC_PROGRAMS: usize = 50;

/// A face of a body, by a point on it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FaceRef {
    pub body: String,
    pub point: [f64; 3],
}

impl FaceRef {
    pub fn is_valid(&self) -> bool {
        !self.body.is_empty() && self.body.len() <= 1000 && self.point.iter().all(|c| c.is_finite() && c.abs() < 1e9)
    }
}

/// What fixes the rotation about the primary axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DirectionRef {
    /// A side plane.
    Side { face: FaceRef },
    /// The line from the first bore to the second.
    Bores { first: FaceRef, second: FaceRef },
}

/// What fixes the origin along one axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OriginRef {
    Side {
        face: FaceRef,
    },
    /// The middle between two opposite side planes.
    Middle {
        first: FaceRef,
        second: FaceRef,
    },
    /// A bore axis.
    Bore {
        face: FaceRef,
    },
}

/// The alignment (3-2-1): primary plane (spatial axis and Z), direction (rotation about Z),
/// origin in X and Y.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Alignment {
    pub primary: Option<FaceRef>,
    pub direction: Option<DirectionRef>,
    pub origin_x: Option<OriginRef>,
    pub origin_y: Option<OriginRef>,
    /// How the alignment's bores (origin, direction) are measured.
    #[serde(skip_serializing_if = "AlignBores::is_cylinder")]
    pub bores: AlignBores,
}

/// How a bore of the alignment is measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignBores {
    /// A cylinder over the depth; the origin is where its axis meets the primary plane.
    #[default]
    Cylinder,
    /// A circle just below the opening (a circle characteristic on the bore gives its points
    /// and height); the origin is its centre.
    Circle,
}

impl AlignBores {
    pub fn is_cylinder(&self) -> bool {
        *self == AlignBores::Cylinder
    }
}

impl Alignment {
    pub fn is_empty(&self) -> bool {
        self.primary.is_none() && self.direction.is_none() && self.origin_x.is_none() && self.origin_y.is_none()
    }
    pub fn is_complete(&self) -> bool {
        self.primary.is_some() && self.direction.is_some() && self.origin_x.is_some() && self.origin_y.is_some()
    }
}

/// The machine side of a setup (the measuring software's machine and probe system): the postprocessor
/// that writes its plan, the probe configuration, the plan's name and where it is written.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeSystem {
    /// The probe configuration (a probe table name or a probe file); `None`: the post's
    /// default. The postprocessor and the plan output belong to the setup's NC programs.
    pub probe_file: Option<String>,
}

/// An NC program of a setup: one output of its inspection plan, through one postprocessor
/// (one measuring machine). A setup can have several, e.g. one per machine.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NcProgram {
    pub id: u64,
    /// "NC Program 1", …
    pub name: String,
    /// The postprocessor (a program in `<data>/inspection/post`, by name or path); `None`: the
    /// first one there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post: Option<String>,
    /// Plan name; empty: from the design and setup names (and this program's, after the first).
    pub plan_name: String,
    /// Where plans are written; `None`: `<data>/inspection/output`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    /// The post's settings (`key → text`), as the post describes them.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub options: std::collections::BTreeMap<String, String>,
    /// The characteristics it outputs (ids); `None`: all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristics: Option<Vec<u64>>,
    /// No setup sheet (PDF) beside the plan; `false` (the default): write it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_sheet: bool,
    /// No CAD model (STEP in the plan's base system) beside the plan; `false` (the default):
    /// write it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_step: bool,
}

impl NcProgram {
    /// A new program with the defaults: the first post, the automatic plan name, the
    /// standard output folder, the post's default settings and every characteristic.
    pub fn new(id: u64, name: String) -> NcProgram {
        NcProgram { id, name, ..Default::default() }
    }
    /// Does it output characteristic `id`?
    pub fn outputs(&self, id: u64) -> bool {
        self.characteristics.as_ref().is_none_or(|c| c.contains(&id))
    }
}

/// A measuring axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn letter(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }
}

fn four() -> u32 {
    4
}

fn seven() -> u32 {
    7
}

fn two() -> u32 {
    2
}

fn two_mm() -> f64 {
    2.0
}

/// Most points picked on one face.
pub const MAX_PICKS: usize = 100;

/// How a plane is probed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaneProbing {
    /// Single points: picked on the face, or spread over it.
    #[default]
    Points,
    /// Circular scan paths placed on the face.
    Circles,
}

/// How a cylinder (bore) is probed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CylinderProbing {
    /// Circular scans along the depth.
    #[default]
    Scan,
    /// Circular paths of single points along the depth.
    CirclePoints,
    /// Single points picked on the wall.
    Points,
}

/// The plane the probe retracts to between elements (a safety plane or group).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyPlane {
    /// The side the probed face looks to.
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "+x")]
    PlusX,
    #[serde(rename = "-x")]
    MinusX,
    #[serde(rename = "+y")]
    PlusY,
    #[serde(rename = "-y")]
    MinusY,
    #[default]
    #[serde(rename = "+z")]
    PlusZ,
    #[serde(rename = "-z")]
    MinusZ,
}

impl SafetyPlane {
    pub const ALL: [SafetyPlane; 7] = [
        SafetyPlane::PlusZ,
        SafetyPlane::Auto,
        SafetyPlane::PlusX,
        SafetyPlane::MinusX,
        SafetyPlane::PlusY,
        SafetyPlane::MinusY,
        SafetyPlane::MinusZ,
    ];
    /// `+Z`, `-X`, … (`None` for automatic).
    pub fn label(self) -> Option<&'static str> {
        match self {
            SafetyPlane::Auto => None,
            SafetyPlane::PlusX => Some("+X"),
            SafetyPlane::MinusX => Some("-X"),
            SafetyPlane::PlusY => Some("+Y"),
            SafetyPlane::MinusY => Some("-Y"),
            SafetyPlane::PlusZ => Some("+Z"),
            SafetyPlane::MinusZ => Some("-Z"),
        }
    }
    /// From `auto`, `+z`, `-X`, …
    pub fn parse(s: &str) -> Option<SafetyPlane> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("auto") {
            return Some(SafetyPlane::Auto);
        }
        SafetyPlane::ALL.into_iter().find(|p| p.label().is_some_and(|l| l.eq_ignore_ascii_case(s)))
    }
}

/// What a characteristic measures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CharacteristicKind {
    /// A circle in a bore, `height` below its opening (the plane it starts at), probed with
    /// `points` single points: its diameter, optionally its position (X, Y) from the origin.
    #[serde(alias = "diameter_points")]
    Circle {
        face: FaceRef,
        #[serde(default = "four")]
        points: u32,
        #[serde(default = "two_mm")]
        height: f64,
        #[serde(default)]
        position: bool,
    },
    /// A bore measured as a cylinder: its diameter.
    #[serde(alias = "diameter_scan")]
    Cylinder {
        face: FaceRef,
        #[serde(default)]
        strategy: CylinderProbing,
        /// Circular paths along the depth (scan and circle points).
        #[serde(default = "two")]
        paths: u32,
        /// Points per path (circle points).
        #[serde(default = "seven")]
        points: u32,
        /// Gap of the first and last path from the bore's ends (mm).
        #[serde(default = "two_mm")]
        height: f64,
        /// Points picked on the wall (strategy points).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        picks: Vec<[f64; 3]>,
    },
    /// How a plane is measured (its strategy holds wherever the face is used: alignment,
    /// distances); evaluates nothing itself.
    Plane {
        face: FaceRef,
        #[serde(default)]
        strategy: PlaneProbing,
        /// Points spread over the face when none are picked.
        #[serde(default = "four")]
        points: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        picks: Vec<[f64; 3]>,
    },
    /// Distance between two elements (planes or bores) along an axis (`None`: the one they are
    /// apart along).
    Distance {
        from: FaceRef,
        to: FaceRef,
        #[serde(default)]
        axis: Option<Axis>,
    },
    /// Angle between two planes.
    Angle { first: FaceRef, second: FaceRef },
    /// A group of points on the part (one, or many along a contour), each checked against the
    /// model: its deviation along the surface normal (the form of a contour), or its X, Y, Z
    /// from the origin. One tolerance for all. Older designs saved one point as `face`.
    Point {
        #[serde(alias = "face", deserialize_with = "one_or_many")]
        points: Vec<FaceRef>,
        /// Older designs: coordinates.
        #[serde(default = "PointCheck::coordinates")]
        check: PointCheck,
    },
}

/// How the points of a point group are evaluated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointCheck {
    /// The deviation of each point along the surface normal (nominal 0): the form of a contour.
    #[default]
    Normal,
    /// The X, Y and Z of each point from the origin.
    Coordinates,
}

impl PointCheck {
    fn coordinates() -> PointCheck {
        PointCheck::Coordinates
    }
}

/// At most this many points in one point group.
pub const MAX_POINTS: usize = 500;

/// One face reference or a list of them (`Point` saved one as `face` before point groups).
fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<FaceRef>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(FaceRef),
        Many(Vec<FaceRef>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(f) => vec![f],
        OneOrMany::Many(v) => v.into_iter().take(MAX_POINTS).collect(),
    })
}

impl CharacteristicKind {
    /// The command that makes (and edits) this kind.
    pub fn command(&self) -> &'static str {
        match self {
            CharacteristicKind::Circle { .. } => "cmm.circle",
            CharacteristicKind::Cylinder { .. } => "cmm.cylinder",
            CharacteristicKind::Plane { .. } => "cmm.plane",
            CharacteristicKind::Distance { .. } => "cmm.distance",
            CharacteristicKind::Angle { .. } => "cmm.angle",
            CharacteristicKind::Point { .. } => "cmm.point",
        }
    }
    /// The faces it uses, in order.
    pub fn faces(&self) -> Vec<&FaceRef> {
        match self {
            CharacteristicKind::Circle { face, .. } | CharacteristicKind::Cylinder { face, .. } | CharacteristicKind::Plane { face, .. } => {
                vec![face]
            }
            CharacteristicKind::Point { points, .. } => points.iter().collect(),
            CharacteristicKind::Distance { from, to, .. } => vec![from, to],
            CharacteristicKind::Angle { first, second } => vec![first, second],
        }
    }
}

fn default_tol() -> f64 {
    0.05
}

/// One characteristic of an inspection plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Characteristic {
    pub id: u64,
    /// Given name; empty: an automatic one from what it measures.
    #[serde(default)]
    pub name: String,
    #[serde(flatten)]
    pub kind: CharacteristicKind,
    /// ± tolerance (millimetres; degrees for an angle).
    #[serde(default = "default_tol")]
    pub tol: f64,
    /// The plane the probe retracts to around its elements; `None`: the setup's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_plane: Option<SafetyPlane>,
    /// The probe configuration (file) measuring its elements; `None`: the setup's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_file: Option<String>,
}

/// One setup: one clamping of the part on the machine, measured with one probe system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub probe: ProbeSystem,
    #[serde(default, skip_serializing_if = "Alignment::is_empty")]
    pub alignment: Alignment,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub characteristics: Vec<Characteristic>,
    /// The plane the probe retracts to between elements (a safety plane or group).
    #[serde(default)]
    pub safety_plane: SafetyPlane,
    /// How planes are probed unless a plane measurement says otherwise.
    #[serde(default)]
    pub plane_probing: PlaneProbing,
    /// Its NC programs (outputs through a postprocessor).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<NcProgram>,
}

impl Setup {
    pub fn new(id: u64, name: String) -> Setup {
        Setup {
            id,
            name,
            probe: ProbeSystem::default(),
            alignment: Alignment::default(),
            characteristics: Vec::new(),
            safety_plane: SafetyPlane::default(),
            plane_probing: PlaneProbing::default(),
            programs: Vec::new(),
        }
    }
    /// "<base> N" with the first N not taken (`base`: "NC Program" in the user's language).
    pub fn next_program_name(&self, base: &str) -> String {
        (1..=MAX_NC_PROGRAMS + 1)
            .map(|n| format!("{base} {n}"))
            .find(|n| !self.programs.iter().any(|p| &p.name == n))
            .unwrap_or_else(|| base.to_string())
    }
}

/// The design's CMM plans.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CmmPlans {
    pub setups: Vec<Setup>,
    /// The setup new characteristics go into.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<u64>,
}

impl CmmPlans {
    pub fn is_empty(&self) -> bool {
        self.setups.is_empty()
    }
    pub fn setup(&self, id: u64) -> Option<&Setup> {
        self.setups.iter().find(|s| s.id == id)
    }
    pub fn setup_mut(&mut self, id: u64) -> Option<&mut Setup> {
        self.setups.iter_mut().find(|s| s.id == id)
    }
    /// The active setup, else the last one.
    pub fn active_setup(&self) -> Option<&Setup> {
        self.active.and_then(|a| self.setup(a)).or_else(|| self.setups.last())
    }
    /// An id no setup, characteristic or NC program has.
    pub fn next_id(&self) -> u64 {
        self.setups
            .iter()
            .flat_map(|s| std::iter::once(s.id).chain(s.characteristics.iter().map(|c| c.id)).chain(s.programs.iter().map(|p| p.id)))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }
    /// "Setup N" with the first N not taken.
    pub fn next_setup_name(&self) -> String {
        (1..=MAX_SETUPS + 1).map(|n| format!("Setup {n}")).find(|n| !self.setups.iter().any(|s| &s.name == n)).unwrap_or_else(|| "Setup".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_round_trips_through_json() {
        let face = |x: f64| FaceRef { body: "Klotz".into(), point: [x, 0.0, 0.0] };
        let plans = CmmPlans {
            setups: vec![Setup {
                probe: ProbeSystem { probe_file: Some("Probe_A".into()) },
                alignment: Alignment { primary: Some(face(0.0)), ..Default::default() },
                characteristics: vec![
                    Characteristic {
                        id: 2,
                        name: String::new(),
                        kind: CharacteristicKind::Cylinder {
                            face: face(1.0),
                            strategy: CylinderProbing::CirclePoints,
                            paths: 2,
                            points: 7,
                            height: 2.0,
                            picks: Vec::new(),
                        },
                        tol: 0.02,
                        safety_plane: None,
                        probe_file: None,
                    },
                    Characteristic {
                        id: 3,
                        name: "A".into(),
                        kind: CharacteristicKind::Distance { from: face(1.0), to: face(2.0), axis: Some(Axis::Y) },
                        tol: 0.1,
                        safety_plane: Some(SafetyPlane::MinusX),
                        probe_file: Some("probe-5.prb".into()),
                    },
                ],
                programs: vec![NcProgram {
                    id: 4,
                    name: "NC-Programm 1".into(),
                    post: Some("my-machine".into()),
                    characteristics: Some(vec![3]),
                    ..Default::default()
                }],
                ..Setup::new(1, "Setup 1".into())
            }],
            active: Some(1),
        };
        let json = serde_json::to_value(&plans).unwrap();
        assert_eq!(json["setups"][0]["characteristics"][1]["type"], "distance");
        assert_eq!(json["setups"][0]["characteristics"][1]["axis"], "y");
        assert_eq!(json["setups"][0]["characteristics"][1]["safety_plane"], "-x");
        assert_eq!(json["setups"][0]["characteristics"][0]["strategy"], "circle_points");
        assert_eq!(json["setups"][0]["safety_plane"], "+z");
        let back: CmmPlans = serde_json::from_value(json).unwrap();
        assert_eq!(back, plans);
        assert_eq!(back.next_id(), 5);
        assert_eq!(back.next_setup_name(), "Setup 2");
        let nc = &back.setups[0].programs[0];
        assert!(nc.outputs(3) && !nc.outputs(2));
        assert_eq!(back.setups[0].next_program_name("NC-Programm"), "NC-Programm 2");
        // older or hand-written plans: defaults fill in, the old kinds read as their successors
        let c: Characteristic =
            serde_json::from_value(serde_json::json!({"id": 9, "type": "diameter_points", "face": {"body": "B", "point": [0, 0, 0]}})).unwrap();
        let b = FaceRef { body: "B".into(), point: [0.0; 3] };
        assert_eq!((c.tol, &c.kind), (0.05, &CharacteristicKind::Circle { face: b.clone(), points: 4, height: 2.0, position: false }));
        let c: Characteristic =
            serde_json::from_value(serde_json::json!({"id": 9, "type": "diameter_scan", "face": {"body": "B", "point": [0, 0, 0]}})).unwrap();
        assert!(matches!(c.kind, CharacteristicKind::Cylinder { strategy: CylinderProbing::Scan, paths: 2, .. }));
        // a point from before point groups: one point, its coordinates
        let c: Characteristic =
            serde_json::from_value(serde_json::json!({"id": 9, "type": "point", "face": {"body": "B", "point": [0, 0, 0]}})).unwrap();
        assert_eq!(c.kind, CharacteristicKind::Point { points: vec![b.clone()], check: PointCheck::Coordinates });
        let back: Characteristic = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        let st: Setup = serde_json::from_value(serde_json::json!({"id": 1, "name": "S"})).unwrap();
        assert_eq!((st.safety_plane, st.plane_probing), (SafetyPlane::PlusZ, PlaneProbing::Points));
        assert_eq!(SafetyPlane::parse("-y"), Some(SafetyPlane::MinusY));
        assert_eq!(SafetyPlane::parse("AUTO"), Some(SafetyPlane::Auto));
        assert_eq!(SafetyPlane::parse("q"), None);
    }
}
