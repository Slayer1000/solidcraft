//! Measurement results read back from the measuring machine, shown at the probed points.
//!
//! `cmm.results` asks the NC program's postprocessor to read the results in the plan's folder
//! (`post --results <folder>`, answer [`solidcraft_inspection::results`]), moves the probed
//! points from the plan's base system into the model and works out, per point, how far it lies
//! from its element's nominal surface (along the outward normal: + is material too much). A point
//! whose element has its own characteristic (a point of a point group) takes that
//! characteristic's value and tolerance, so the view shows what the report shows. View state:
//! not saved, not an undo step.

use crate::tr;
use serde_json::{Value, json};
use solidcraft_inspection::program::{BaseSystem, Element, Geometry, Program};
use solidcraft_inspection::results as res;
pub use solidcraft_inspection::results::{Along, CharacteristicResult, Results, State};

/// How a point's deviation compares with its tolerance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    /// Within ¾ of the tolerance.
    Good,
    /// Within the tolerance, in its last quarter.
    Near,
    /// Outside the tolerance.
    Out,
    /// No tolerance (or no deviation) known.
    Unknown,
}

/// One probed point, in model coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultPoint {
    /// The program element it was probed for.
    pub element: String,
    /// Where the probe touched the surface, and its probing direction (into the material).
    pub at: [f64; 3],
    pub dir: [f64; 3],
    /// Distance from the nominal surface along the outward normal (mm), if the element's
    /// geometry is known.
    pub deviation: Option<f64>,
    /// The allowed deviations (upper ≥ lower), when a characteristic gives them.
    pub tolerance: Option<(f64, f64)>,
    /// The characteristic the deviation and tolerance come from.
    pub characteristic: Option<String>,
}

impl ResultPoint {
    pub fn grade(&self) -> Grade {
        let (Some(d), Some((upper, lower))) = (self.deviation, self.tolerance) else { return Grade::Unknown };
        if d > upper + 1e-12 || d < lower - 1e-12 {
            return Grade::Out;
        }
        let mid = (upper + lower) / 2.0;
        let half = (upper - lower) / 2.0;
        if (d - mid).abs() <= 0.75 * half + 1e-12 { Grade::Good } else { Grade::Near }
    }
}

/// Results loaded for the view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadedResults {
    pub setup: u64,
    /// The NC program (none: the setup's defaults).
    pub nc: Option<u64>,
    pub plan: String,
    /// The folder they were read from.
    pub folder: String,
    pub measured: Option<String>,
    pub points: Vec<ResultPoint>,
    pub characteristics: Vec<CharacteristicResult>,
}

impl LoadedResults {
    /// Characteristics in and out of tolerance.
    pub fn counts(&self) -> (usize, usize) {
        let inside = self.characteristics.iter().filter(|c| c.state == State::In).count();
        let out = self.characteristics.iter().filter(|c| c.state == State::Out).count();
        (inside, out)
    }

    /// For `cmm.results` and the tree.
    pub fn describe(&self) -> Value {
        let (inside, out) = self.counts();
        let grade = |g: Grade| self.points.iter().filter(|p| p.grade() == g).count();
        json!({
            "setup": self.setup,
            "nc": self.nc,
            "plan": self.plan,
            "folder": self.folder,
            "measured": self.measured,
            "points": self.points.len(),
            "points_good": grade(Grade::Good),
            "points_near": grade(Grade::Near),
            "points_out": grade(Grade::Out),
            "characteristics": self.characteristics.iter().map(|c| json!({
                "name": c.name,
                "actual": c.actual,
                "nominal": c.nominal,
                "upper": c.upper,
                "lower": c.lower,
                "unit": c.unit,
                "state": match c.state { State::In => "in", State::Out => "out", State::Unknown => "unknown" },
            })).collect::<Vec<_>>(),
            "in_tolerance": inside,
            "out_of_tolerance": out,
        })
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A base-system direction in the model.
fn model_direction(base: &BaseSystem, v: [f64; 3]) -> [f64; 3] {
    add(add(scale(base.x, v[0]), scale(base.y, v[1])), scale(base.z, v[2]))
}

/// A base-system point in the model.
fn model_point(base: &BaseSystem, p: [f64; 3]) -> [f64; 3] {
    add(base.origin, model_direction(base, p))
}

/// Distance of the axis-normal component of `p − origin` from an axis through `origin`.
fn radial(p: [f64; 3], origin: [f64; 3], axis: [f64; 3]) -> f64 {
    let d = sub(p, origin);
    let along = dot(d, axis);
    let r = sub(d, scale(axis, along));
    dot(r, r).sqrt()
}

/// How far `p` lies outside the nominal surface of `g` (outward: + is material too much).
fn surface_deviation(g: &Geometry, p: [f64; 3]) -> Option<f64> {
    let unit = |v: [f64; 3]| {
        let l = dot(v, v).sqrt();
        (l.is_finite() && l > 1e-12).then(|| scale(v, 1.0 / l))
    };
    let d = match g {
        Geometry::Plane { normal, point, .. } | Geometry::Point { point, normal } => dot(sub(p, *point), unit(*normal)?),
        Geometry::Cylinder { top, axis, diameter, inner, .. } => {
            let r = radial(p, *top, unit(*axis)?);
            if *inner { diameter / 2.0 - r } else { r - diameter / 2.0 }
        }
        Geometry::Circle { center, normal, diameter, inner } => {
            let r = radial(p, *center, unit(*normal)?);
            if *inner { diameter / 2.0 - r } else { r - diameter / 2.0 }
        }
    };
    d.is_finite().then_some(d)
}

/// A point's deviation and tolerance from the characteristic the post names as its own
/// (`element`, `along`): a deviation along the normal as it is; a coordinate (the one along which
/// the point's normal runs) turned into the deviation along the outward normal.
fn own_characteristic<'a>(r: &'a Results, e: &Element, base: &BaseSystem) -> Option<(&'a CharacteristicResult, (f64, (f64, f64)))> {
    let Geometry::Point { normal, .. } = &e.geometry else { return None };
    let n = base.direction(*normal);
    let mine = || r.characteristics.iter().filter(|c| c.unit == "mm" && c.element.as_deref() == Some(e.name.as_str()));
    if let Some(c) = mine().find(|c| c.along == Some(Along::Normal)) {
        return Some((c, (c.actual - c.nominal, (c.upper, c.lower))));
    }
    mine().find_map(|c| {
        let axis = match c.along? {
            Along::X => 0,
            Along::Y => 1,
            Along::Z => 2,
            Along::Normal => return None,
        };
        let along = n.get(axis).copied().filter(|x| x.abs() > 0.5)?;
        let d = c.actual - c.nominal;
        Some((c, if along > 0.0 { (d, (c.upper, c.lower)) } else { (-d, (-c.lower, -c.upper)) }))
    })
}

/// Places `results` on the model through the program they were measured with.
pub fn place(program: &Program, results: &Results) -> std::result::Result<Vec<ResultPoint>, String> {
    place_on(&program.elements, program.base_system, results)
}

/// [`place`] with the program's elements and base system.
pub fn place_on(elements: &[Element], base: Option<BaseSystem>, results: &Results) -> std::result::Result<Vec<ResultPoint>, String> {
    results.validate()?;
    let base =
        base.filter(BaseSystem::is_valid).unwrap_or(BaseSystem { origin: [0.0; 3], x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0] });
    let mut out = Vec::new();
    for er in &results.elements {
        let element = elements.iter().find(|e| e.name == er.name);
        let own = element.and_then(|e| own_characteristic(results, e, &base));
        for p in er.points.iter().take(res::MAX_ITEMS) {
            if out.len() >= res::MAX_ITEMS {
                break;
            }
            let at = model_point(&base, p.at);
            let dir = model_direction(&base, p.dir);
            let (deviation, tolerance, characteristic) = match (&own, element) {
                (Some((c, (d, tol))), _) => (Some(*d), Some(*tol), Some(c.name.clone())),
                (None, Some(e)) => (surface_deviation(&e.geometry, at), None, None),
                (None, None) => (None, None, None),
            };
            out.push(ResultPoint { element: er.name.clone(), at, dir, deviation, tolerance, characteristic });
        }
    }
    Ok(out)
}

/// Parses a post's `--results` answer.
pub fn parse(answer: Value) -> std::result::Result<Results, String> {
    let r: Results = serde_json::from_value(answer).map_err(|e| tr!("Measurement results: {e}", e = e))?;
    r.validate()?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_inspection::program::Strategy;
    use solidcraft_inspection::results::{ElementResult, FORMAT, ProbedPoint};

    fn element(name: &str, geometry: Geometry) -> Element {
        Element { name: name.into(), geometry, strategy: Strategy::Points { count: 1, at: Vec::new() }, safety_plane: None, probe_file: None }
    }

    fn characteristic(name: &str, element: &str, along: Along, actual: f64, nominal: f64, state: State) -> CharacteristicResult {
        CharacteristicResult {
            name: name.into(),
            actual,
            nominal,
            upper: 0.05,
            lower: -0.05,
            unit: "mm".into(),
            state,
            element: Some(element.into()),
            along: Some(along),
        }
    }

    #[test]
    fn results_land_on_the_model_with_their_deviations() {
        // base system: origin (10, 20, 30), axes turned 90° about Z (base X = model Y)
        let base = BaseSystem { origin: [10.0, 20.0, 30.0], x: [0.0, 1.0, 0.0], y: [-1.0, 0.0, 0.0], z: [0.0, 0.0, 1.0] };
        let elements = vec![
            element("Top", Geometry::Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, 30.0], min: [0.0; 3], max: [0.0; 3] }),
            element("Bore", Geometry::Cylinder { top: [10.0, 20.0, 30.0], axis: [0.0, 0.0, -1.0], diameter: 10.0, depth: 5.0, inner: true }),
            // a point on a side looking to model −Y = base −X
            element("P1", Geometry::Point { point: [10.0, 15.0, 25.0], normal: [0.0, -1.0, 0.0] }),
            element("P2", Geometry::Point { point: [0.0, 0.0, 0.0], normal: [0.0, 0.0, 1.0] }),
        ];
        let results = Results {
            format: FORMAT.into(),
            plan: "P".into(),
            measured: None,
            elements: vec![
                ElementResult { name: "Top".into(), points: vec![ProbedPoint { at: [1.0, 2.0, 0.02], dir: [0.0, 0.0, -1.0] }] },
                // 4.99 from the axis: the bore is 0.01 too small (material too much)
                ElementResult { name: "Bore".into(), points: vec![ProbedPoint { at: [4.99, 0.0, -2.0], dir: [1.0, 0.0, 0.0] }] },
                ElementResult { name: "P1".into(), points: vec![ProbedPoint { at: [-5.06, 0.0, -5.0], dir: [1.0, 0.0, 0.0] }] },
                ElementResult { name: "P2".into(), points: vec![ProbedPoint { at: [0.0, 0.0, 0.0], dir: [0.0, 0.0, -1.0] }] },
                ElementResult { name: "Unknown".into(), points: vec![ProbedPoint { at: [0.0; 3], dir: [0.0, 0.0, -1.0] }] },
            ],
            characteristics: vec![
                // base X of P1: nominal −5, measured −5.06 → 0.06 outward (base −X): out
                characteristic("X of P1", "P1", Along::X, -5.06, -5.0, State::Out),
                characteristic("Deviation of P2", "P2", Along::Normal, 0.03, 0.0, State::In),
                // named like P2 but not its own: ignored
                characteristic("Size P2", "", Along::Normal, 9.0, 0.0, State::Out),
            ],
        };
        let pts = place_on(&elements, Some(base), &results).unwrap();
        assert_eq!(pts.len(), 5);
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9);
        // base (1, 2, 0.02) → model (10 − 2, 20 + 1, 30.02)
        assert!(close(pts[0].at, [8.0, 21.0, 30.02]), "{:?}", pts[0].at);
        assert!((pts[0].deviation.unwrap() - 0.02).abs() < 1e-9);
        assert_eq!(pts[0].grade(), Grade::Unknown);
        assert!(close(pts[1].dir, [0.0, 1.0, 0.0]));
        assert!((pts[1].deviation.unwrap() - 0.01).abs() < 1e-9);
        assert!((pts[2].deviation.unwrap() - 0.06).abs() < 1e-9, "{:?}", pts[2]);
        assert_eq!(pts[2].characteristic.as_deref(), Some("X of P1"));
        assert_eq!(pts[2].grade(), Grade::Out);
        assert!((pts[3].deviation.unwrap() - 0.03).abs() < 1e-9);
        assert_eq!(pts[3].grade(), Grade::Good);
        assert_eq!(pts[4].deviation, None);
        // 0.045 of ±0.05: in its last quarter
        let near = ResultPoint { deviation: Some(0.045), tolerance: Some((0.05, -0.05)), ..pts[4].clone() };
        assert_eq!(near.grade(), Grade::Near);
    }

    #[test]
    fn hostile_results_are_refused() {
        let bad = json!({"format": FORMAT, "elements": [{"name": "E", "points": [{"at": ["x", 0, 0], "dir": [0, 0, 1]}]}]});
        assert!(parse(bad).is_err());
        assert!(parse(json!({"format": "other/1"})).is_err());
        assert!(parse(json!([1, 2])).is_err());
        let r = Results { format: FORMAT.into(), ..Default::default() };
        assert_eq!(place_on(&[], None, &r).unwrap(), Vec::new());
        let huge = Results {
            format: FORMAT.into(),
            elements: vec![ElementResult { name: "E".into(), points: vec![ProbedPoint { at: [f64::NAN, 0.0, 0.0], dir: [0.0; 3] }] }],
            ..Default::default()
        };
        assert!(place_on(&[], None, &huge).is_err());
    }
}
