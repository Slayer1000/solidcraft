//! Measurement results read back from the measuring machine (`solidcraft-cmm-results/1`).
//!
//! The postprocessor reads its software's result files (`post --results <plan folder>`) and
//! answers with these results: per measured element the probed points on the part's surface,
//! per characteristic the actual value with its nominal and tolerance. Coordinates are in the
//! plan's base system (the program's `base_system`), like the nominal values handed to the post.
//! Units: mm; angles in degrees.

use serde::{Deserialize, Serialize};

/// The format tag of [`Results`].
pub const FORMAT: &str = "solidcraft-cmm-results/1";
/// Upper limit for elements, characteristics and points per element.
pub const MAX_ITEMS: usize = 20_000;

/// One measuring run's results.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Results {
    pub format: String,
    /// The plan's name.
    pub plan: String,
    /// When the run started (ISO 8601, local time of the machine), if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<String>,
    pub elements: Vec<ElementResult>,
    pub characteristics: Vec<CharacteristicResult>,
}

/// The points probed on one element (its name as in the program).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ElementResult {
    pub name: String,
    pub points: Vec<ProbedPoint>,
}

/// One probed point: where the probe touched the surface and the direction it probed in (into
/// the material).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProbedPoint {
    pub at: [f64; 3],
    pub dir: [f64; 3],
}

/// Whether an actual value is within its tolerance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    In,
    Out,
    #[default]
    Unknown,
}

/// What a characteristic of a single element measures, so that its value can be shown at the
/// element's probed point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Along {
    /// The deviation along the element's outward surface normal (nominal 0; + is material too
    /// much).
    Normal,
    /// One coordinate of the element in the plan's base system.
    X,
    Y,
    Z,
}

/// A characteristic's result (its name as the post wrote it in the plan).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacteristicResult {
    pub name: String,
    pub actual: f64,
    pub nominal: f64,
    /// Allowed deviations from the nominal (upper ≥ lower).
    pub upper: f64,
    pub lower: f64,
    /// `mm` or `deg`.
    pub unit: String,
    pub state: State,
    /// The program element this characteristic evaluates on its own (a point of a point group),
    /// if any: its value is then shown at that element's probed point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    /// What it measures of `element`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub along: Option<Along>,
}

impl Results {
    /// The format, finite numbers and sizes within [`MAX_ITEMS`].
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("results format {:?} is not {FORMAT}", self.format));
        }
        if self.elements.len() > MAX_ITEMS || self.characteristics.len() > MAX_ITEMS {
            return Err("too many elements or characteristics".into());
        }
        let finite = |p: &[f64; 3]| p.iter().all(|c| c.is_finite());
        for e in &self.elements {
            if e.points.len() > MAX_ITEMS || !e.points.iter().all(|p| finite(&p.at) && finite(&p.dir)) {
                return Err(format!("element {:?}: too many points or a number that is not finite", e.name));
            }
        }
        for c in &self.characteristics {
            if c.element.as_ref().is_some_and(|e| e.len() > 1000) {
                return Err(format!("characteristic {:?}: element name too long", c.name));
            }
            if ![c.actual, c.nominal, c.upper, c.lower].iter().all(|x| x.is_finite()) {
                return Err(format!("characteristic {:?}: a number that is not finite", c.name));
            }
        }
        Ok(())
    }

    pub fn element(&self, name: &str) -> Option<&ElementResult> {
        self.elements.iter().find(|e| e.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_round_trip_and_are_checked() {
        let r = Results {
            format: FORMAT.into(),
            plan: "P".into(),
            measured: Some("2026-10-27T11:34:20".into()),
            elements: vec![ElementResult { name: "E".into(), points: vec![ProbedPoint { at: [1.0, 2.0, 3.0], dir: [0.0, 0.0, -1.0] }] }],
            characteristics: vec![CharacteristicResult {
                name: "D".into(),
                actual: 10.01,
                nominal: 10.0,
                upper: 0.02,
                lower: -0.02,
                unit: "mm".into(),
                state: State::In,
                element: Some("E".into()),
                along: Some(Along::Normal),
            }],
        };
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains(r#""along":"normal""#), "{text}");
        // older answers without element and along still read
        let plain: Results = serde_json::from_str(&text.replace(r#","element":"E","along":"normal""#, "")).unwrap();
        assert_eq!(plain.characteristics[0].element, None);
        let back: Results = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
        assert!(r.validate().is_ok());
        let mut bad = r.clone();
        bad.elements[0].points[0].at[0] = f64::NAN;
        assert!(bad.validate().is_err());
        bad = r;
        bad.format = "x".into();
        assert!(bad.validate().is_err());
    }
}
