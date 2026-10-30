//! A complete, minimal CMM postprocessor: the starting point for a post for another measuring
//! machine (see `docs/cmm-postprocessors.md`).
//!
//! It answers the three calls SolidCraft makes:
//!
//! - `example_post --describe`: its settings (shown in the NC program dialog) and that it can
//!   read results back.
//! - `example_post <program.json> <output folder> [--replace]`: writes the plan. This example
//!   writes a readable measuring list `<output folder>/<plan>/plan.txt` with every coordinate in
//!   the plan's base system; a real post writes its machine software's plan format instead.
//! - `example_post --results <plan folder>`: reads the results back. This example reads a simple
//!   text file `results.txt` in the plan folder (format below); a real post reads its machine
//!   software's result files.
//!
//! `results.txt`, one record per line, fields separated by `;`, coordinates in the base system:
//!
//! ```text
//! point;<element>;<x>;<y>;<z>;<dx>;<dy>;<dz>        surface point and probing direction
//! char;<name>;<actual>;<nominal>;<upper>;<lower>;<mm|deg>[;<element>;<normal|x|y|z>]
//! ```
//!
//! Build and install (Linux): `cargo build --release -p solidcraft-inspection --example
//! example_post`, then copy `target/release/examples/example_post` into
//! `~/.local/share/solidcraft/inspection/post/` (Windows: `example_post.exe` into
//! `%LOCALAPPDATA%\SolidCraft\inspection\post\`).

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use serde_json::json;
use solidcraft_inspection::program::{BaseSystem, Characteristic, CharacteristicKind, Direction, Element, Geometry, Origin, Program, Strategy};
use solidcraft_inspection::results::{self, Along, CharacteristicResult, ElementResult, ProbedPoint, Results, State};

/// The settings this post offers (`--describe`). SolidCraft passes the chosen values back in
/// `program.post_options` (`bool` as `on` / `off`).
fn describe() -> serde_json::Value {
    json!({
        "name": "example",
        "options": [
            {"key": "title", "label": "Title line", "kind": "text", "default": ""},
            {"key": "with_points", "label": "List every probing point", "kind": "bool", "default": "on"},
        ],
        "results": results::FORMAT,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let answer = match args.first().map(String::as_str) {
        Some("--describe") => Ok(describe()),
        Some("--results") => match args.get(1) {
            Some(folder) => read_results(Path::new(folder)),
            None => Err("usage: example_post --results <plan folder>".to_string()),
        },
        Some(program) => match args.get(1) {
            Some(out) => write_plan(Path::new(program), Path::new(out), args.iter().any(|a| a == "--replace")),
            None => Err("usage: example_post <program.json> <output folder> [--replace]".to_string()),
        },
        None => Err("usage: example_post --describe | <program.json> <output folder> [--replace] | --results <plan folder>".to_string()),
    };
    match answer {
        // the answer: one JSON object, the last line on stdout
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        // a failure: the reason on stderr, a non-zero exit code
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

// ------------------------------------------------------------------------------------------
// Writing the plan
// ------------------------------------------------------------------------------------------

/// The identity: for a program without a base system the model's coordinates are used as they are.
const MODEL: BaseSystem = BaseSystem { origin: [0.0; 3], x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0] };

fn xyz(p: [f64; 3]) -> String {
    format!("X {:.4}  Y {:.4}  Z {:.4}", p[0], p[1], p[2])
}

fn write_plan(program: &Path, out: &Path, replace: bool) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(program).map_err(|e| format!("{}: {e}", program.display()))?;
    let p: Program = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", program.display()))?;
    // what every post may rely on: the format, unique names, finite numbers, sizes
    p.validate()?;
    let plan_name: String = p.plan.chars().map(|c| if c.is_alphanumeric() || "-_.,+ ".contains(c) { c } else { '_' }).collect();
    let folder = out.join(plan_name.trim());
    if folder.exists() && !replace {
        return Err(format!("{} exists already (export with replace to overwrite it)", folder.display()));
    }
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;

    // the plan's software keeps its coordinates in the base system: move everything there
    let base = p.base_system.unwrap_or(MODEL);
    let with_points = p.post_options.get("with_points").is_none_or(|v| v != "off");
    let mut s = String::new();
    let mut skipped = Vec::new();
    if let Some(title) = p.post_options.get("title").filter(|t| !t.trim().is_empty()) {
        let _ = writeln!(s, "{title}");
    }
    let _ = writeln!(s, "Plan {}  (setup {})", p.plan, p.setup);
    let _ = writeln!(s, "Probe {}", p.probe_file.as_deref().unwrap_or("(machine default)"));
    let _ = writeln!(s, "\nAlignment");
    let _ = writeln!(s, "  primary plane  {}", p.alignment.primary);
    let direction = match &p.alignment.direction {
        Direction::Side { plane } => format!("plane {plane}"),
        Direction::Line { from, to } => format!("line {from} → {to}"),
    };
    let _ = writeln!(s, "  direction      {direction}");
    for (axis, o) in [("X", &p.alignment.origin_x), ("Y", &p.alignment.origin_y)] {
        let what = match o {
            Origin::Side { plane } => format!("plane {plane}"),
            Origin::Middle { first, second } => format!("middle of {first} and {second}"),
            Origin::Axis { element } => format!("axis of {element}"),
        };
        let _ = writeln!(s, "  origin {axis}       {what}");
    }
    let _ = writeln!(s, "\nElements");
    for e in &p.elements {
        element(&mut s, e, &base, with_points);
    }
    let _ = writeln!(s, "\nCharacteristics");
    for c in &p.characteristics {
        match characteristic(c) {
            Some(line) => {
                let _ = writeln!(s, "  {line}");
            }
            // a kind this post cannot write yet: say so instead of failing
            None => skipped.push(c.name.clone()),
        }
    }
    let file = folder.join("plan.txt");
    std::fs::write(&file, s).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(json!({"folder": folder.display().to_string(), "skipped": skipped, "warnings": []}))
}

fn element(s: &mut String, e: &Element, base: &BaseSystem, with_points: bool) {
    let n = |v: [f64; 3]| xyz(base.direction(v));
    let at = |v: [f64; 3]| xyz(base.point(v));
    let what = match &e.geometry {
        Geometry::Plane { normal, point, .. } => format!("plane through {} normal {}", at(*point), n(*normal)),
        Geometry::Cylinder { top, axis, diameter, depth, inner } => {
            format!("{} Ø{diameter:.4} top {} axis {} depth {depth:.3}", if *inner { "bore" } else { "pin" }, at(*top), n(*axis))
        }
        Geometry::Circle { center, normal, diameter, inner } => {
            format!("{} circle Ø{diameter:.4} at {} axis {}", if *inner { "inner" } else { "outer" }, at(*center), n(*normal))
        }
        Geometry::Point { point, normal } => format!("point {} normal {}", at(*point), n(*normal)),
    };
    let safety = e.safety_plane.as_deref().unwrap_or("auto");
    let _ = writeln!(s, "  {}: {what}  [safety {safety}]", e.name);
    let how = match &e.strategy {
        Strategy::CirclesOnPlane { circles } => format!("{} circular paths", circles.len()),
        Strategy::CylinderScan { paths, step, margin } => format!("{paths} scans, {step} mm apart along the path, {margin} mm from the ends"),
        Strategy::CylinderPoints { paths, points, margin } => format!("{paths} circles of {points} points, {margin} mm from the ends"),
        Strategy::Points { count, at: list } => {
            if with_points {
                for (i, q) in list.iter().enumerate() {
                    let _ = writeln!(s, "      P{} {}", i + 1, at(*q));
                }
            }
            format!("{count} points")
        }
    };
    let _ = writeln!(s, "      probing: {how}");
}

fn characteristic(c: &Characteristic) -> Option<String> {
    let tol = format!("+{:.4} / {:.4}", c.tol.upper, c.tol.lower);
    Some(match &c.kind {
        CharacteristicKind::Diameter { element, nominal } => format!("{}: diameter of {element} = {nominal:.4} {tol}", c.name),
        CharacteristicKind::Distance { from, to, axis, nominal } => {
            format!("{}: distance {from} → {to} along {} = {nominal:.4} {tol}", c.name, axis.letter())
        }
        CharacteristicKind::Angle { first, second, nominal } => format!("{}: angle {first} / {second} = {nominal:.4}° {tol}", c.name),
        CharacteristicKind::Coordinate { element, axis, nominal } => format!("{}: {} of {element} = {nominal:.4} {tol}", c.name, axis.letter()),
        CharacteristicKind::Deviation { element } => format!("{}: deviation of {element} along its normal {tol}", c.name),
    })
}

// ------------------------------------------------------------------------------------------
// Reading results back
// ------------------------------------------------------------------------------------------

fn number(fields: &[&str], i: usize, line: usize) -> Result<f64, String> {
    let f = fields.get(i).ok_or_else(|| format!("results.txt line {line}: field {} is missing", i + 1))?;
    f.trim().replace(',', ".").parse::<f64>().ok().filter(|x| x.is_finite()).ok_or_else(|| format!("results.txt line {line}: {f:?} is not a number"))
}

fn read_results(folder: &Path) -> Result<serde_json::Value, String> {
    let file = folder.join("results.txt");
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: no results ({e})", file.display()))?;
    let plan = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = Results { format: results::FORMAT.into(), plan, ..Default::default() };
    for (i, raw) in text.lines().enumerate().take(200_000) {
        let line = i + 1;
        let f: Vec<&str> = raw.split(';').map(str::trim).collect();
        match f.first().copied() {
            Some("point") => {
                let name = f.get(1).copied().unwrap_or_default().to_string();
                let v = |k| number(&f, k, line);
                let p = ProbedPoint { at: [v(2)?, v(3)?, v(4)?], dir: [v(5)?, v(6)?, v(7)?] };
                match out.elements.iter_mut().find(|e| e.name == name) {
                    Some(e) => e.points.push(p),
                    None => out.elements.push(ElementResult { name, points: vec![p] }),
                }
            }
            Some("char") => {
                let v = |k| number(&f, k, line);
                let (actual, nominal, upper, lower) = (v(2)?, v(3)?, v(4)?, v(5)?);
                let (upper, lower) = (upper.max(lower), lower.min(upper));
                let d = actual - nominal;
                let along = match f.get(8).copied() {
                    Some("normal") => Some(Along::Normal),
                    Some("x") => Some(Along::X),
                    Some("y") => Some(Along::Y),
                    Some("z") => Some(Along::Z),
                    _ => None,
                };
                out.characteristics.push(CharacteristicResult {
                    name: f.get(1).copied().unwrap_or_default().to_string(),
                    actual,
                    nominal,
                    upper,
                    lower,
                    unit: if f.get(6).copied() == Some("deg") { "deg" } else { "mm" }.into(),
                    state: if d <= upper && d >= lower { State::In } else { State::Out },
                    element: f.get(7).filter(|e| !e.is_empty()).map(|e| e.to_string()),
                    along,
                });
            }
            Some("") | None => {}
            Some(other) => return Err(format!("results.txt line {line}: unknown record {other:?}")),
        }
    }
    out.validate()?;
    serde_json::to_value(&out).map_err(|e| e.to_string())
}
