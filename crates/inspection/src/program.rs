//! The neutral CMM program (`solidcraft-cmm-program/1`): what one setup measures, independent
//! of any measuring machine or its software. SolidCraft writes it; a **postprocessor** (a
//! separate program outside this repository, one per machine software) translates it into that
//! software's own plan, as a CAM post translates tool paths into a machine's NC code.
//!
//! Everything is in millimetres and degrees, in the part's model coordinates (Z up). The
//! alignment says how the base system (the part coordinate system) is built from measured
//! elements; characteristics give their nominal values in that base system. A post writes what
//! it supports and reports the rest as skipped.
//!
//! The parameters of each function are documented here and in `docs/cmm-postprocessors.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The format tag of this version.
pub const FORMAT: &str = "solidcraft-cmm-program/1";

/// Most elements or characteristics a program holds (posts may rely on it).
pub const MAX_ITEMS: usize = 2000;

/// One setup's measuring program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// [`FORMAT`].
    pub format: String,
    /// The plan's name (a folder name for most machine software).
    pub plan: String,
    /// The setup it comes from.
    #[serde(default)]
    pub setup: String,
    /// The probe configuration file the setup chose (its format is the post's business).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_file: Option<String>,
    /// Ball diameters (mm) from the probe table, by the `probe_file` text of the setup or an
    /// element. A `probe_file` may also be a bare configuration name (no file): the post then
    /// knows the probe only by this name and ball (for example a single vertical stylus).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub probe_balls: BTreeMap<String, f64>,
    /// Settings for the post (`key → text`), passed through unchanged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub post_options: BTreeMap<String, String>,
    /// The measured elements, in measuring order (alignment elements first).
    pub elements: Vec<Element>,
    /// How the base system is built.
    pub alignment: Alignment,
    /// What is evaluated, in order.
    #[serde(default)]
    pub characteristics: Vec<Characteristic>,
    /// The box around the part the probe must stay out of between elements.
    pub safety: Safety,
    /// Where the alignment puts the base system, in model coordinates (origin and unit axes),
    /// from the nominal geometry. Machine software that keeps its nominals in the base system
    /// moves every coordinate of the program into it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_system: Option<BaseSystem>,
}

/// A coordinate system in model coordinates: origin and unit axes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaseSystem {
    pub origin: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub z: [f64; 3],
}

impl BaseSystem {
    /// A model point in this system.
    pub fn point(&self, p: [f64; 3]) -> [f64; 3] {
        let d = [p[0] - self.origin[0], p[1] - self.origin[1], p[2] - self.origin[2]];
        self.direction(d)
    }
    /// A model direction in this system.
    pub fn direction(&self, v: [f64; 3]) -> [f64; 3] {
        let dot = |a: [f64; 3]| a[0] * v[0] + a[1] * v[1] + a[2] * v[2];
        [dot(self.x), dot(self.y), dot(self.z)]
    }
    /// Unit axes, at right angles, right-handed, finite.
    pub fn is_valid(&self) -> bool {
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let cross = [
            self.x[1] * self.y[2] - self.x[2] * self.y[1],
            self.x[2] * self.y[0] - self.x[0] * self.y[2],
            self.x[0] * self.y[1] - self.x[1] * self.y[0],
        ];
        self.origin.iter().all(|c| c.is_finite())
            && [self.x, self.y, self.z].iter().all(|a| (dot(*a, *a) - 1.0).abs() < 1e-6)
            && dot(self.x, self.y).abs() < 1e-6
            && dot(cross, self.z) > 1.0 - 1e-6
    }
}

/// A measured element: its nominal geometry and how it is probed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Element {
    /// Unique within the program; characteristics and the alignment refer to it.
    pub name: String,
    pub geometry: Geometry,
    pub strategy: Strategy,
    /// The safety plane the probe retracts to before and after the element: one of
    /// [`SAFETY_PLANES`]; `None`: the post's choice (usually the side the face looks to).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_plane: Option<String>,
    /// Its own probe configuration file (a characteristic chose another probe); `None`: the
    /// program's `probe_file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_file: Option<String>,
}

/// The safety planes an element may name.
pub const SAFETY_PLANES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];

fn two_mm() -> f64 {
    2.0
}

/// Nominal geometry of an element.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Geometry {
    /// A plane: outward unit normal (away from the material), a point on it, and the bounding
    /// box of the face (zero thickness along an axis-parallel normal).
    Plane { normal: [f64; 3], point: [f64; 3], min: [f64; 3], max: [f64; 3] },
    /// A cylinder: `top` is the axis point where it starts (a bore: at its opening), `axis` the
    /// unit direction into it (a vertical bore from above: [0, 0, −1]); `inner`: a bore (the
    /// material is outside), else a pin or outer radius.
    Cylinder { top: [f64; 3], axis: [f64; 3], diameter: f64, depth: f64, inner: bool },
    /// A circle (a section of a cylinder): centre, the axis normal, diameter.
    Circle { center: [f64; 3], normal: [f64; 3], diameter: f64, inner: bool },
    /// A single point on a surface, with the surface's outward normal there.
    Point { point: [f64; 3], normal: [f64; 3] },
}

/// How an element is probed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Strategy {
    /// Circular scan paths on a plane: centre and diameter of each.
    CirclesOnPlane { circles: Vec<PathCircle> },
    /// Circular scans along a cylinder's depth: `paths` circles, `step` mm between points; the
    /// first and last path `margin` mm from the cylinder's ends.
    CylinderScan {
        paths: u32,
        step: f64,
        #[serde(default = "two_mm")]
        margin: f64,
    },
    /// Circular paths of single points along a cylinder's depth: `paths` circles of `points`
    /// points each, evenly around; the first and last path `margin` mm from the ends.
    CylinderPoints {
        paths: u32,
        points: u32,
        #[serde(default = "two_mm")]
        margin: f64,
    },
    /// Single points: `at` lists them (on the surface, model coordinates) where SolidCraft
    /// placed them; without it the post spreads `count` over the element (on a circle: evenly
    /// around it). The probing direction at each point is against the surface normal (into
    /// the material).
    Points {
        count: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        at: Vec<[f64; 3]>,
    },
}

/// One circular path of [`Strategy::CirclesOnPlane`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathCircle {
    pub center: [f64; 3],
    pub diameter: f64,
}

/// The base system (3-2-1): primary plane (spatial axis and Z), direction (rotation about Z),
/// origin in X and Y.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alignment {
    /// An upward plane.
    pub primary: String,
    pub direction: Direction,
    pub origin_x: Origin,
    pub origin_y: Origin,
}

/// What fixes the rotation about the primary axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Direction {
    /// A side plane: the axis along it (a ±Y face gives X, a ±X face gives Y).
    Side { plane: String },
    /// The line from the first cylinder's axis to the second's: +X.
    Line { from: String, to: String },
}

/// What fixes the origin along one axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Origin {
    Side {
        plane: String,
    },
    /// The middle (symmetry plane) of two opposite side planes.
    Middle {
        first: String,
        second: String,
    },
    /// A cylinder's axis.
    Axis {
        element: String,
    },
}

/// A measuring axis of the base system.
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

/// A tolerance: the allowed deviations from the nominal (upper ≥ lower).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    pub upper: f64,
    pub lower: f64,
}

impl Tolerance {
    /// ± `t`.
    pub fn symmetric(t: f64) -> Tolerance {
        Tolerance { upper: t, lower: -t }
    }
}

/// An evaluated characteristic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Characteristic {
    pub name: String,
    #[serde(flatten)]
    pub kind: CharacteristicKind,
    pub tol: Tolerance,
}

/// What a characteristic evaluates. Nominals in mm, angles in degrees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CharacteristicKind {
    /// Diameter of a cylinder or circle.
    Diameter { element: String, nominal: f64 },
    /// Distance between two elements along an axis of the base system (centre to centre; a
    /// plane by its position along its normal).
    Distance { from: String, to: String, axis: Axis, nominal: f64 },
    /// Angle between two elements (planes: between their normals).
    Angle { first: String, second: String, nominal: f64 },
    /// A coordinate of an element (a circle's centre, a point) in the base system.
    Coordinate { element: String, axis: Axis, nominal: f64 },
    /// The deviation of a point element along its normal (nominal 0): one point of a contour.
    Deviation { element: String },
}

/// The box around the part, and how far outside it the probe travels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Safety {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub clearance: f64,
}

impl Program {
    pub fn element(&self, name: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.name == name)
    }

    /// Checks what every post may rely on: the format tag, unique names, references that
    /// exist, finite numbers, and sizes within [`MAX_ITEMS`].
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("format {:?} is not {FORMAT}", self.format));
        }
        if self.elements.len() > MAX_ITEMS || self.characteristics.len() > MAX_ITEMS {
            return Err("too many elements or characteristics".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for e in &self.elements {
            if e.name.trim().is_empty() || !names.insert(e.name.as_str()) {
                return Err(format!("element name {:?} is empty or repeated", e.name));
            }
            let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
            let ok = match &e.geometry {
                Geometry::Plane { normal, point, min, max } => finite(normal) && finite(point) && finite(min) && finite(max),
                Geometry::Cylinder { top, axis, diameter, depth, .. } => finite(top) && finite(axis) && *diameter > 0.0 && depth.is_finite(),
                Geometry::Circle { center, normal, diameter, .. } => finite(center) && finite(normal) && *diameter > 0.0,
                Geometry::Point { point, normal } => finite(point) && finite(normal),
            };
            let ok = ok
                && match &e.strategy {
                    Strategy::CirclesOnPlane { circles } => {
                        circles.len() <= MAX_ITEMS && circles.iter().all(|c| finite(&c.center) && c.diameter > 0.0)
                    }
                    Strategy::CylinderScan { paths, step, margin } => *paths >= 1 && *step > 0.0 && *margin >= 0.0 && margin.is_finite(),
                    Strategy::CylinderPoints { paths, points, margin } => *paths >= 1 && *points >= 3 && *margin >= 0.0 && margin.is_finite(),
                    Strategy::Points { at, .. } => at.len() <= MAX_ITEMS && at.iter().all(|p| finite(p)),
                };
            if !ok {
                return Err(format!("element {:?} has a number that is not finite or not positive", e.name));
            }
            if let Some(sp) = &e.safety_plane
                && !SAFETY_PLANES.contains(&sp.as_str())
            {
                return Err(format!("element {:?}: safety plane {sp:?} is not one of {SAFETY_PLANES:?}", e.name));
            }
        }
        if self.probe_balls.len() > MAX_ITEMS || self.probe_balls.values().any(|b| !b.is_finite() || *b <= 0.0 || *b > 1000.0) {
            return Err("probe ball diameters must be positive mm".into());
        }
        if self.base_system.is_some_and(|b| !b.is_valid()) {
            return Err("the base system needs an origin and three unit axes at right angles".into());
        }
        if self.base_system.is_some_and(|b| !b.is_valid()) {
            return Err("the base system needs an origin and three unit axes at right angles".into());
        }
        let known = |n: &str| -> Result<(), String> { if names.contains(n) { Ok(()) } else { Err(format!("no element {n:?}")) } };
        let a = &self.alignment;
        known(&a.primary)?;
        match &a.direction {
            Direction::Side { plane } => known(plane)?,
            Direction::Line { from, to } => {
                known(from)?;
                known(to)?;
            }
        }
        for o in [&a.origin_x, &a.origin_y] {
            match o {
                Origin::Side { plane } => known(plane)?,
                Origin::Middle { first, second } => {
                    known(first)?;
                    known(second)?;
                }
                Origin::Axis { element } => known(element)?,
            }
        }
        for c in &self.characteristics {
            match &c.kind {
                CharacteristicKind::Diameter { element, .. }
                | CharacteristicKind::Coordinate { element, .. }
                | CharacteristicKind::Deviation { element } => known(element)?,
                CharacteristicKind::Distance { from, to, .. } => {
                    known(from)?;
                    known(to)?;
                }
                CharacteristicKind::Angle { first, second, .. } => {
                    known(first)?;
                    known(second)?;
                }
            }
            if !(c.tol.upper.is_finite() && c.tol.lower.is_finite() && c.tol.upper >= c.tol.lower) {
                return Err(format!("characteristic {:?}: the tolerance must have upper ≥ lower", c.name));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(name: &str, normal: [f64; 3]) -> Element {
        Element {
            name: name.into(),
            geometry: Geometry::Plane { normal, point: [0.0; 3], min: [0.0; 3], max: [1.0, 1.0, 0.0] },
            strategy: Strategy::CirclesOnPlane { circles: vec![PathCircle { center: [0.5, 0.5, 0.0], diameter: 0.5 }] },
            safety_plane: Some("+Z".into()),
            probe_file: None,
        }
    }

    fn program() -> Program {
        Program {
            format: FORMAT.into(),
            plan: "P".into(),
            setup: "Setup 1".into(),
            probe_file: None,
            post_options: BTreeMap::new(),
            elements: vec![
                plane("Top", [0.0, 0.0, 1.0]),
                plane("Front", [0.0, -1.0, 0.0]),
                Element {
                    name: "Bore".into(),
                    geometry: Geometry::Cylinder { top: [0.0; 3], axis: [0.0, 0.0, -1.0], diameter: 10.0, depth: 20.0, inner: true },
                    strategy: Strategy::CylinderPoints { paths: 2, points: 7, margin: 2.0 },
                    safety_plane: None,
                    probe_file: Some("probe-5.prb".into()),
                },
            ],
            alignment: Alignment {
                primary: "Top".into(),
                direction: Direction::Side { plane: "Front".into() },
                origin_x: Origin::Axis { element: "Bore".into() },
                origin_y: Origin::Side { plane: "Front".into() },
            },
            characteristics: vec![Characteristic {
                name: "D".into(),
                kind: CharacteristicKind::Diameter { element: "Bore".into(), nominal: 10.0 },
                tol: Tolerance::symmetric(0.02),
            }],
            safety: Safety { min: [0.0; 3], max: [1.0; 3], clearance: 10.0 },
            base_system: Some(BaseSystem { origin: [0.0, 0.0, -71.0], x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0] }),
            probe_balls: BTreeMap::new(),
        }
    }

    #[test]
    fn a_program_round_trips_and_validates() {
        let p = program();
        p.validate().unwrap();
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["elements"][2]["geometry"]["type"], "cylinder");
        assert_eq!(json["characteristics"][0]["type"], "diameter");
        assert_eq!(json["alignment"]["origin_x"]["type"], "axis");
        let back: Program = serde_json::from_value(json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn broken_programs_are_refused() {
        let mut p = program();
        p.alignment.primary = "None".into();
        assert!(p.validate().is_err());
        let mut p = program();
        p.elements.push(plane("Top", [0.0, 0.0, 1.0]));
        assert!(p.validate().is_err());
        let mut p = program();
        p.characteristics[0].tol = Tolerance { upper: -1.0, lower: 1.0 };
        assert!(p.validate().is_err());
        let mut p = program();
        p.format = "other/1".into();
        assert!(p.validate().is_err());
        let mut p = program();
        p.base_system = Some(BaseSystem { origin: [0.0; 3], x: [1.0, 0.0, 0.0], y: [1.0, 0.0, 0.0], z: [0.0, 0.0, 1.0] });
        assert!(p.validate().is_err());
        let b = program().base_system.unwrap();
        assert_eq!(b.point([5.0, -3.0, -88.0]), [5.0, -3.0, -17.0]);
        let mut p = program();
        if let Some(e) = p.elements.first_mut() {
            e.safety_plane = Some("up".into());
        }
        assert!(p.validate().is_err());
        let mut p = program();
        if let Some(e) = p.elements.get_mut(2) {
            e.strategy = Strategy::CylinderPoints { paths: 0, points: 7, margin: 2.0 };
        }
        assert!(p.validate().is_err());
        // older programs: a scan without margin gets 2 mm
        let s: Strategy = serde_json::from_value(serde_json::json!({"type": "cylinder_scan", "paths": 2, "step": 0.1})).unwrap();
        assert_eq!(s, Strategy::CylinderScan { paths: 2, step: 0.1, margin: 2.0 });
    }
}
