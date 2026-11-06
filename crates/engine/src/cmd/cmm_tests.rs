use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    match s.execute(cmd, &p) {
        Ok(v) => v,
        Err(e) => panic!("{cmd} {p}: {e}"),
    }
}

/// The project's test block: 100 × 60 × 30 below Z 0, five through bores, two blind holes.
fn block() -> Session {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": {"origin": [0, 0, -30], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}, "name": "Grund"}));
    run(&mut s, "sketch.rectangle.center", json!({"center": [0, 0], "corner": [50, 30]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"distance": 30, "body_name": "Klotz"}));
    run(&mut s, "sketch.create", json!({"plane": {"origin": [0, 0, 0], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}, "name": "Durch"}));
    for (x, y, d) in [(0, 0, 20), (-35, -15, 10), (35, -15, 10), (35, 15, 10), (-35, 15, 10)] {
        run(&mut s, "sketch.circle.center", json!({"center": [x, y], "diameter": d}));
    }
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"through_all": true, "direction": "negative", "operation": "cut", "profiles": "all"}));
    run(&mut s, "sketch.create", json!({"plane": {"origin": [0, 0, 0], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}, "name": "Sack"}));
    for x in [-20, 20] {
        run(&mut s, "sketch.circle.center", json!({"center": [x, 0], "diameter": 6}));
    }
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"distance": 12, "direction": "negative", "operation": "cut", "profiles": "all"}));
    s
}

fn face(p: [f64; 3]) -> Value {
    json!({"body": "Klotz", "point": p})
}

const TOP: [f64; 3] = [-45.0, -25.0, 0.0];
const FRONT: [f64; 3] = [0.0, -30.0, -15.0];
const BACK: [f64; 3] = [0.0, 30.0, -15.0];
const LEFT: [f64; 3] = [-50.0, 0.0, -15.0];
const RIGHT: [f64; 3] = [50.0, 0.0, -15.0];
const BORE_M: [f64; 3] = [-10.0, 0.0, -10.0];
const BORE_B1: [f64; 3] = [-40.0, -15.0, -10.0];
const BORE_B2: [f64; 3] = [30.0, -15.0, -10.0];
const BLIND_S1: [f64; 3] = [-23.0, 0.0, -5.0];

#[test]
fn faces_are_recognised() {
    let s = block();
    let m = s.world_state().unwrap();
    let a = |p: [f64; 3]| crate::inspection::analyze(&m, &crate::inspection::FaceRef { body: "Klotz".into(), point: p }).unwrap();
    match a(BORE_M).kind {
        crate::inspection::FaceKind::Bore { x, y, diameter, top, depth } => {
            assert!(x.abs() < 1e-6 && y.abs() < 1e-6, "{x} {y}");
            assert!((diameter - 20.0).abs() < 1e-6 && top.abs() < 1e-6 && (depth - 30.0).abs() < 1e-6, "{diameter} {top} {depth}");
        }
        k => panic!("{k:?}"),
    }
    match a(BLIND_S1).kind {
        crate::inspection::FaceKind::Bore { diameter, depth, .. } => assert!((diameter - 6.0).abs() < 1e-6 && (depth - 12.0).abs() < 1e-6),
        k => panic!("{k:?}"),
    }
    // both halves of a bore are one face
    assert!(a(BORE_B1).same(&a([-30.0, -15.0, -20.0])));
    match a(FRONT).kind {
        crate::inspection::FaceKind::Plane { normal, min, max } => {
            assert_eq!(normal, crate::inspection::Normal::MinusY);
            assert_eq!(min, [-50.0, -30.0, -30.0]);
            assert_eq!(max, [50.0, -30.0, 0.0]);
        }
        k => panic!("{k:?}"),
    }
    // circles on the top face stay clear of the holes and the edges
    let top = a(TOP);
    let circles = crate::inspection::circles(&top, 4).unwrap();
    assert_eq!(circles.len(), 4, "{circles:?}");
    let holes =
        [(0.0, 0.0, 10.0), (-35.0, -15.0, 5.0), (35.0, -15.0, 5.0), (35.0, 15.0, 5.0), (-35.0, 15.0, 5.0), (-20.0, 0.0, 3.0), (20.0, 0.0, 3.0)];
    for (c, d) in &circles {
        let r = d / 2.0;
        assert!(c[2].abs() < 1e-9);
        assert!(c[0] - r >= -50.0 && c[0] + r <= 50.0 && c[1] - r >= -30.0 && c[1] + r <= 30.0, "{c:?} {d}");
        for (hx, hy, hr) in holes {
            let dist = ((c[0] - hx).powi(2) + (c[1] - hy).powi(2)).sqrt();
            // the path runs beside the hole or around it, never over its edge
            assert!(dist + hr < r || dist - hr > r, "circle {c:?} Ø{d} crosses the hole at ({hx}, {hy})");
        }
    }
}

fn setup_with_alignment(s: &mut Session) -> u64 {
    let r = run(s, "cmm.setup", json!({}));
    let id = r["setup"].as_u64().unwrap();
    // two bores for the direction, the middle bore for the origin (X and Y)
    let st = run(s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(BORE_B1), face(BORE_B2)], "origin_x": [face(BORE_M)]}));
    assert_eq!(st["alignment"]["complete"], true, "{}", st["alignment"]["problem"]);
    id
}

#[test]
fn setups_hold_an_alignment_and_characteristics() {
    let mut s = block();
    assert!(s.execute("cmm.alignment", &json!({})).is_err(), "no setup yet");
    let id = setup_with_alignment(&mut s);
    assert_eq!(s.doc.cmm.active, Some(id));
    assert_eq!(s.doc.cmm.setups[0].name, "Setup 1");
    let st = run(&mut s, "cmm.state", json!({}));
    assert_eq!(st["setups"][0]["alignment"]["frame"]["origin"], json!([0.0, 0.0, 0.0]));
    assert_eq!(st["setups"][0]["alignment"]["frame"]["x"], json!([1.0, 0.0, 0.0]));

    // the classic 3-2-1: a side for the direction, the middle of the sides for X and Y
    run(&mut s, "cmm.alignment", json!({"direction": [face(FRONT)], "origin_x": [face(LEFT), face(RIGHT)], "origin_y": [face(FRONT)]}));
    let st = run(&mut s, "cmm.state", json!({}));
    assert_eq!(st["setups"][0]["alignment"]["frame"]["origin"], json!([0.0, -30.0, 0.0]));

    // plane + side + bore, the bore picked twice (both halves of its wall): the bore is the
    // origin in X and Y (a test part of 23.10.2026)
    run(&mut s, "cmm.alignment", json!({"direction": [face(FRONT)], "origin_x": [face(BORE_M), face([10.0, 0.0, -20.0])], "origin_y": []}));
    let st = run(&mut s, "cmm.state", json!({}));
    assert_eq!(st["setups"][0]["alignment"]["complete"], json!(true), "{st}");
    assert_eq!(st["setups"][0]["alignment"]["frame"]["origin"], json!([0.0, 0.0, 0.0]), "{st}");
    run(&mut s, "cmm.alignment", json!({"direction": [face(FRONT)], "origin_x": [face(LEFT), face(RIGHT)], "origin_y": [face(FRONT)]}));

    let add = |s: &mut Session, cmd: &str, p: Value| run(s, cmd, p)["characteristic"].as_u64().unwrap();
    let d3 = add(&mut s, "cmm.circle", json!({"face": face(BORE_B2), "position": true, "tol": 0.02}));
    let scan = add(&mut s, "cmm.cylinder", json!({"face": face(BORE_M), "name": "Mittelbohrung"}));
    let dist = add(&mut s, "cmm.distance", json!({"from": face(FRONT), "to": face(BACK)}));
    let dist_x = add(&mut s, "cmm.distance", json!({"from": face(BORE_B1), "to": face(BORE_B2)}));
    let ang = add(&mut s, "cmm.angle", json!({"first": face(TOP), "second": face(FRONT)}));
    let pt = add(&mut s, "cmm.point", json!({"face": face([10.0, 20.0, 0.0])}));
    let st = run(&mut s, "cmm.state", json!({}));
    let cs = st["setups"][0]["characteristics"].as_array().unwrap().clone();
    assert_eq!(cs.len(), 6);
    let by = |id: u64| cs.iter().find(|c| c["id"] == id).unwrap().clone();
    assert_eq!(by(d3)["value"], 10.0);
    // the position is in the part coordinates: origin at the front, in the middle of X
    assert!(by(d3)["nominal"].as_str().unwrap().contains("X35 Y15"), "{}", by(d3)["nominal"]);
    assert_eq!(by(scan)["name"], "Mittelbohrung");
    assert_eq!((by(dist)["value"].as_f64(), by(dist)["nominal"].as_str()), (Some(60.0), Some("60 ±0.05 (Y)")));
    assert_eq!(by(dist_x)["value"], 70.0);
    assert_eq!(by(ang)["value"], 90.0);
    assert_eq!(by(pt)["value"], json!([10.0, 50.0, 0.0]));

    // edit in place, rename, delete; undo brings it back
    run(&mut s, "cmm.circle", json!({"characteristic": d3, "face": face(BORE_B2), "points": 5, "tol": 0.01}));
    let c = s.doc.cmm.setups[0].characteristics.iter().find(|c| c.id == d3).unwrap().clone();
    assert_eq!(
        (c.tol, &c.kind),
        (0.01, &crate::inspection::CharacteristicKind::Circle { face: c.kind.faces()[0].clone(), points: 5, height: 2.0, position: false })
    );
    run(&mut s, "cmm.rename", json!({"characteristic": d3, "name": "Passung"}));
    assert_eq!(s.doc.cmm.setups[0].characteristics[0].name, "Passung");
    run(&mut s, "cmm.characteristic.delete", json!({"characteristic": d3}));
    assert_eq!(s.doc.cmm.setups[0].characteristics.len(), 5);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.cmm.setups[0].characteristics.len(), 6);

    // a second setup becomes active and takes the next characteristics
    let second = run(&mut s, "cmm.setup", json!({"name": "Rückseite"}))["setup"].as_u64().unwrap();
    assert_eq!((s.doc.cmm.active, s.doc.cmm.setups[1].name.as_str()), (Some(second), "Rückseite"));
    add(&mut s, "cmm.cylinder", json!({"face": face(BORE_B1)}));
    assert_eq!(s.doc.cmm.setups[1].characteristics.len(), 1);
    run(&mut s, "cmm.setup.activate", json!({"setup": id}));
    run(&mut s, "cmm.setup.delete", json!({"setup": second}));
    assert_eq!(s.doc.cmm.setups.len(), 1);

    // saved with the design, without evaluating the model again
    let evaluations = s.evaluations;
    run(&mut s, "cmm.rename", json!({"setup": id, "name": "Oben"}));
    assert_eq!(s.evaluations, evaluations, "a plan change does not rebuild the model");
    let saved = serde_json::to_value(&*s.doc).unwrap();
    assert_eq!(saved["cmm"]["setups"][0]["name"], "Oben");
    let back: solidcraft_doc::Document = serde_json::from_value(saved).unwrap();
    assert_eq!(back.cmm, s.doc.cmm);
}

#[test]
fn wrong_faces_are_refused() {
    let mut s = block();
    run(&mut s, "cmm.setup", json!({}));
    let err = |s: &mut Session, cmd: &str, p: Value| assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
    // primary must face up; direction a side or two bores; the origin across its axis
    err(&mut s, "cmm.alignment", json!({"primary": face(FRONT), "direction": [face(LEFT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    err(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(TOP)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    err(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(FRONT)], "origin_y": [face(FRONT)]}));
    err(
        &mut s,
        "cmm.alignment",
        json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(LEFT), face(LEFT)], "origin_y": [face(FRONT)]}),
    );
    err(&mut s, "cmm.alignment", json!({"primary": {"body": "Klotz", "point": [500, 0, 0]}}));
    err(&mut s, "cmm.cylinder", json!({"face": face(TOP)}));
    err(&mut s, "cmm.distance", json!({"from": face(FRONT), "to": face(LEFT)}));
    err(&mut s, "cmm.distance", json!({"from": face(FRONT), "to": face(FRONT)}));
    err(&mut s, "cmm.distance", json!({"from": face(FRONT), "to": face(BACK), "axis": "x"}));
    err(&mut s, "cmm.angle", json!({"first": face(FRONT), "second": face(BACK)}));
    err(&mut s, "cmm.angle", json!({"first": face(BORE_M), "second": face(TOP)}));
    err(&mut s, "cmm.export", json!({}));
    for (cmd, p) in [
        ("cmm.setup", json!({"name": 5})),
        ("cmm.setup", json!({"setup": 999})),
        ("cmm.setup", json!({"probe_file": "/gibt/es/nicht.prb"})),
        ("cmm.setup", json!({"options": {"a": 1}})),
        ("cmm.circle", json!({"face": face(BORE_M), "points": 2})),
        ("cmm.circle", json!({"face": face(BORE_M), "height": 30})),
        ("cmm.circle", json!({"face": face(BORE_M), "safety_plane": "oben"})),
        ("cmm.cylinder", json!({"face": face(BORE_M), "strategy": "spiral"})),
        ("cmm.cylinder", json!({"face": face(BORE_M), "height": 15})),
        ("cmm.cylinder", json!({"face": face(BORE_M), "strategy": "points", "picks": [[10, 0, -5]]})),
        ("cmm.plane", json!({"face": face(BORE_M)})),
        ("cmm.plane", json!({"face": face(TOP), "picks": [[-45, -25, 0], [45, -25, 0], [45, 25, 0]]})),
        ("cmm.plane", json!({"face": face(TOP), "picks": [[-45, -25, 0], [45, -25, 0], [45, 25, 0], [0, -30, -15]]})),
        ("cmm.plane", json!({"face": face(TOP), "strategy": "zigzag"})),
        ("cmm.setup", json!({"plane_probing": "lines"})),
        ("cmm.cylinder", json!({"face": face(BORE_M), "tol": -1})),
        ("cmm.cylinder", json!({"face": "x"})),
        ("cmm.cylinder", json!({"face": {"body": "Klotz", "point": [f64::MAX, 0, 0]}})),
        ("cmm.cylinder", json!({"face": face(BORE_M), "characteristic": 999})),
        ("cmm.distance", json!({"from": face(FRONT), "to": face(BACK), "axis": "w"})),
        ("cmm.alignment", json!({"direction": [face(FRONT), face(BACK), face(LEFT)]})),
        ("cmm.setup.delete", json!({})),
        ("cmm.setup.activate", json!({"setup": "a"})),
        ("cmm.characteristic.delete", json!({"characteristic": 1})),
        ("cmm.rename", json!({"name": ""})),
    ] {
        err(&mut s, cmd, p);
    }
    assert!(s.doc.cmm.setups[0].characteristics.is_empty() && s.doc.cmm.setups[0].alignment.is_empty());
}

/// A setup with every kind of characteristic becomes a neutral program that names each
/// element once, carries the alignment, and gives nominals in the part coordinates.
#[test]
fn a_setup_becomes_a_neutral_program() {
    use solidcraft_inspection::program::{CharacteristicKind as K, Direction, Geometry, Origin, Strategy};
    let mut s = block();
    run(&mut s, "cmm.setup", json!({}));
    run(&mut s, "cmm.nc", json!({"plan_name": "SC_Klotz_Setup", "options": {"base": "x"}}));
    run(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    run(&mut s, "cmm.cylinder", json!({"face": face(BORE_M), "tol": 0.02}));
    run(&mut s, "cmm.cylinder", json!({"face": face(BLIND_S1)}));
    run(&mut s, "cmm.distance", json!({"from": face(BORE_B1), "to": face(BORE_B2)}));
    run(&mut s, "cmm.distance", json!({"from": face(FRONT), "to": face(BACK)}));
    run(&mut s, "cmm.angle", json!({"first": face(TOP), "second": face(FRONT)}));
    run(&mut s, "cmm.circle", json!({"face": face(BORE_B2), "position": true, "points": 4}));
    run(&mut s, "cmm.point", json!({"face": face([10.0, 20.0, 0.0]), "check": "coordinates"}));
    let v = run(&mut s, "cmm.program", json!({}));
    let p: solidcraft_inspection::program::Program = serde_json::from_value(v).unwrap();
    p.validate().unwrap();
    assert_eq!((p.plan.as_str(), p.setup.as_str()), ("SC_Klotz_Setup", "Setup 1"));
    assert_eq!(p.post_options.get("base").map(String::as_str), Some("x"));
    // alignment elements first: top, front, left
    assert_eq!(p.alignment.primary, "Plane +Z Z0");
    assert_eq!(p.alignment.direction, Direction::Side { plane: "Plane -Y Y-30".into() });
    assert_eq!(p.alignment.origin_x, Origin::Side { plane: "Plane -X X-50".into() });
    let names: Vec<&str> = p.elements.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names.get(..3), Some(&["Plane +Z Z0", "Plane -Y Y-30", "Plane -X X-50"][..]));
    let bore = p.element("Cylinder Ø20 X0 Y0").unwrap();
    assert!(matches!(bore.geometry, Geometry::Cylinder { diameter, depth, inner: true, .. } if diameter == 20.0 && depth == 30.0));
    assert!(matches!(bore.strategy, Strategy::CylinderScan { paths: 2, margin: 2.0, .. }));
    // planes: 4 single points by default, the safety plane +Z
    let top = p.element("Plane +Z Z0").unwrap();
    assert!(matches!(&top.strategy, Strategy::Points { count: 4, at } if at.len() == 4), "{:?}", top.strategy);
    assert_eq!(top.safety_plane.as_deref(), Some("+Z"));
    let kinds: Vec<&K> = p.characteristics.iter().map(|c| &c.kind).collect();
    assert!(kinds.iter().any(|k| matches!(k, K::Diameter { element, nominal } if element == "Cylinder Ø20 X0 Y0" && *nominal == 20.0)));
    assert!(p.characteristics.iter().any(|c| c.tol.upper == 0.02 && c.tol.lower == -0.02));
    assert!(kinds.iter().any(|k| matches!(k, K::Distance { nominal, axis: solidcraft_inspection::program::Axis::Y, .. } if *nominal == 60.0)));
    assert!(kinds.iter().any(|k| matches!(k, K::Angle { nominal, .. } if *nominal == 90.0)));
    // the circle 2 mm below the bore's opening, its 4 points around it, its position from the
    // origin (front left top corner)
    let circle = p.elements.iter().find(|e| matches!(e.geometry, Geometry::Circle { .. })).unwrap();
    assert!(matches!(circle.geometry, Geometry::Circle { center, diameter, .. } if center == [35.0, -15.0, -2.0] && diameter == 10.0));
    assert!(
        matches!(&circle.strategy, Strategy::Points { count: 4, at } if at.first() == Some(&[40.0, -15.0, -2.0]) && at.get(1) == Some(&[35.0, -10.0, -2.0])),
        "{:?}",
        circle.strategy
    );
    let coord = |el: &str, ax: &str| {
        p.characteristics.iter().find_map(|c| match &c.kind {
            K::Coordinate { element, axis, nominal } if element == el && axis.letter() == ax => Some(*nominal),
            _ => None,
        })
    };
    assert_eq!((coord(&circle.name, "X"), coord(&circle.name, "Y")), (Some(85.0), Some(15.0)));
    let point = p.elements.iter().find(|e| matches!(e.geometry, Geometry::Point { .. })).unwrap();
    assert_eq!((coord(&point.name, "X"), coord(&point.name, "Y"), coord(&point.name, "Z")), (Some(60.0), Some(50.0), Some(0.0)));
    assert_eq!((p.safety.min, p.safety.max), ([-50.0, -30.0, -30.0], [50.0, 30.0, 0.0]));
    // the base system the alignment makes: the front left top corner
    let b = p.base_system.unwrap();
    assert_eq!((b.origin, b.x, b.z), ([-50.0, -30.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]));
    assert_eq!(b.point([50.0, 30.0, -30.0]), [100.0, 60.0, -30.0]);
}

/// Plane and cylinder measurements choose how a face is probed wherever it is used; points
/// picked on a face must lie on it; each characteristic may name its own safety plane.
#[test]
fn strategies_and_safety_planes() {
    use solidcraft_inspection::program::Strategy;
    let mut s = block();
    run(&mut s, "cmm.setup", json!({"safety_plane": "auto"}));
    run(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    let picks = json!([[-45, -25, 0], [45, -25, 0], [45, 25, 0], [-45, 25, 0], [0, 25, 0]]);
    let r = run(&mut s, "cmm.plane", json!({"face": face(TOP), "picks": picks}));
    assert!(r["characteristics"][0]["nominal"].as_str().is_some_and(|n| n.contains("5 points (picked)")), "{r}");
    assert_eq!(r["characteristics"][0]["markers"].as_array().map(Vec::len), Some(5));
    run(&mut s, "cmm.plane", json!({"face": face(FRONT), "strategy": "circles", "safety_plane": "-y"}));
    run(&mut s, "cmm.cylinder", json!({"face": face(BORE_M), "strategy": "circle_points", "points": 7, "paths": 3, "height": 3}));
    let wall = json!([[-30, -15, -5], [-40, -15, -5], [-35, -10, -10], [-35, -20, -20]]);
    run(&mut s, "cmm.cylinder", json!({"face": face(BORE_B1), "strategy": "points", "picks": wall}));
    // another probe for one bore: its element names the probe file, the others keep the setup's
    let pcf = std::env::temp_dir().join(format!("sc-probe-{}.prb", std::process::id()));
    std::fs::write(&pcf, b"x").unwrap();
    let pcf_s = pcf.display().to_string();
    run(&mut s, "cmm.cylinder", json!({"face": face(BORE_B2), "probe_file": pcf_s}));
    run(&mut s, "cmm.distance", json!({"from": face(FRONT), "to": face(BACK), "safety_plane": "+z"}));
    let p: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    p.validate().unwrap();
    let e = |n: &str| p.element(n).unwrap_or_else(|| panic!("{n}: {:?}", p.elements.iter().map(|e| &e.name).collect::<Vec<_>>()));
    // the top: the points picked (the alignment uses it too), the setup's safety plane: automatic
    assert!(matches!(&e("Plane +Z Z0").strategy, Strategy::Points { count: 5, at } if at[1] == [45.0, -25.0, 0.0]));
    assert_eq!(e("Plane +Z Z0").safety_plane, None);
    assert!(matches!(&e("Plane -Y Y-30").strategy, Strategy::CirclesOnPlane { circles } if circles.len() == 2));
    // the plane measurement of the front says its safety plane, also where the alignment uses it
    assert_eq!(e("Plane -Y Y-30").safety_plane.as_deref(), Some("-Y"));
    assert_eq!(e("Plane +Y Y30").safety_plane.as_deref(), Some("+Z"));
    assert_eq!(e("Cylinder Ø20 X0 Y0").strategy, Strategy::CylinderPoints { paths: 3, points: 7, margin: 3.0 });
    assert!(matches!(&e("Cylinder Ø10 X-35 Y-15").strategy, Strategy::Points { count: 4, .. }));
    assert_eq!(e("Cylinder Ø10 X35 Y-15").strategy, Strategy::CylinderScan { paths: 2, step: 0.1, margin: 2.0 });
    // the markers show the paths: 3 × 7 points in the middle bore
    let st = run(&mut s, "cmm.state", json!({}));
    let cyl = st["setups"][0]["characteristics"].as_array().unwrap().iter().find(|c| c["type"] == "cylinder").unwrap().clone();
    assert_eq!(cyl["markers"].as_array().map(Vec::len), Some(21), "{cyl}");
    assert_eq!(cyl["markers"][0]["normal"], json!([-1.0, 0.0, 0.0]), "a bore's wall looks to its axis");
    assert!(s.execute("cmm.cylinder", &json!({"face": face(BORE_B2), "probe_file": "/gibt/es/nicht.prb"})).is_err());
    let p: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    assert_eq!(p.element("Cylinder Ø10 X35 Y-15").unwrap().probe_file.as_deref(), Some(pcf_s.as_str()));
    assert_eq!(p.element("Plane +Z Z0").unwrap().probe_file, None);
    let _ = std::fs::remove_file(&pcf);
    // without a plane measurement, a setup may probe its planes with circles
    let id = s.doc.cmm.setups[0].id;
    run(&mut s, "cmm.setup", json!({"setup": id, "plane_probing": "circles"}));
    let p: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    assert!(matches!(&p.element("Plane -X X-50").unwrap().strategy, Strategy::CirclesOnPlane { .. }));
    assert!(matches!(&p.element("Plane +Z Z0").unwrap().strategy, Strategy::Points { count: 5, .. }), "the plane measurement wins");
}

/// Points spread over a plane keep clear of its edges and holes.
#[test]
fn spread_points_avoid_holes_and_edges() {
    let s = block();
    let m = s.world_state().unwrap();
    let top = crate::inspection::analyze(&m, &crate::inspection::FaceRef { body: "Klotz".into(), point: TOP }).unwrap();
    let pts = crate::inspection::spread_points(&top, 6).unwrap();
    assert_eq!(pts.len(), 6);
    let holes =
        [(0.0, 0.0, 10.0), (-35.0, -15.0, 5.0), (35.0, -15.0, 5.0), (35.0, 15.0, 5.0), (-35.0, 15.0, 5.0), (-20.0, 0.0, 3.0), (20.0, 0.0, 3.0)];
    for p in &pts {
        assert!(p[2].abs() < 1e-9 && p[0].abs() <= 47.0 + 1e-9 && p[1].abs() <= 27.0 + 1e-9, "{p:?}");
        for (hx, hy, hr) in holes {
            assert!(((p[0] - hx).powi(2) + (p[1] - hy).powi(2)).sqrt() >= hr + 1.0, "{p:?} too near the hole at ({hx}, {hy})");
        }
    }
    // a narrow wall still gets points
    let front = crate::inspection::analyze(&m, &crate::inspection::FaceRef { body: "Klotz".into(), point: FRONT }).unwrap();
    assert_eq!(crate::inspection::spread_points(&front, 4).unwrap().len(), 4);
}

/// The probe table keeps each probe's dimensions; bad numbers are refused.
#[test]
fn the_probe_table_round_trips() {
    use crate::inspection::{ProbeRow, read_probe_table, write_probe_table};
    let path = std::env::temp_dir().join(format!("sc-probes-{}", std::process::id())).join("probe-table.json");
    let row = ProbeRow {
        file: "Stylus2".into(),
        ball: Some(2.0),
        length: Some(40.0),
        measuring_length: Some(31.0),
        shaft: Some(1.5),
        body: Some(5.0),
        order_no: "123".into(),
        ..Default::default()
    };
    write_probe_table(&path, std::slice::from_ref(&row)).unwrap();
    assert_eq!(read_probe_table(&path), vec![row.clone()]);
    assert!(write_probe_table(&path, &[ProbeRow { ball: Some(-1.0), ..row.clone() }]).is_err());
    assert!(write_probe_table(&path, &[ProbeRow { file: String::new(), ..row }]).is_err());
    assert_eq!(crate::inspection::probe_key("C:\\x\\Probe_A.PRB"), "Probe_A");
    assert_eq!(crate::inspection::probe_key("Stylus2"), "Stylus2");
    assert_eq!(crate::inspection::probe_key("/p/Ø2 L40.5"), "Ø2 L40.5");
    // the choice lists every table row: as its file when there is one, else by name
    let table = [ProbeRow { file: "Probe_B".into(), ..Default::default() }, ProbeRow { file: "Probe_A".into(), ..Default::default() }];
    let files = vec![std::path::PathBuf::from("/t/Probe_A.prb"), std::path::PathBuf::from("/t/Probe_C.PRB")];
    // only the table's rows: a probe file without a row is not offered
    assert_eq!(crate::inspection::choices_of(&table, files), vec!["/t/Probe_A.prb", "Probe_B"]);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// An inclined plane (a 30° chamfer, as on the test part of 22.10.2026) is probed with single
/// points; the angle to the top is 30°.
#[test]
fn an_inclined_plane_is_probed_with_points() {
    use solidcraft_inspection::program::{CharacteristicKind as K, Geometry, Strategy};
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"corner": [0, 0, 0], "length": 80, "width": 40, "height": 40, "body_name": "Testklotz"}));
    run(&mut s, "solid.chamfer", json!({"edges": [[80.0, 15.0, 40.0]], "distance": "10", "angle": "30 deg"}));
    let f = |p: [f64; 3]| json!({"body": "Testklotz", "point": p});
    run(&mut s, "cmm.setup", json!({}));
    run(
        &mut s,
        "cmm.alignment",
        json!({"primary": f([20.0, 20.0, 40.0]), "direction": [f([40.0, 0.0, 20.0])], "origin_x": [f([0.0, 20.0, 20.0])], "origin_y": [f([40.0, 0.0, 20.0])]}),
    );
    let r = run(&mut s, "cmm.angle", json!({"first": f([20.0, 20.0, 40.0]), "second": f([75.0, 20.0, 37.113])}));
    assert!(r["characteristics"][0]["nominal"].as_str().is_some_and(|n| n.starts_with("30°")), "{r}");
    let p: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    let inclined =
        p.elements.iter().find(|e| matches!(e.geometry, Geometry::Plane { normal, .. } if normal[0].abs() > 0.1 && normal[2].abs() > 0.1)).unwrap();
    let Geometry::Plane { normal, .. } = inclined.geometry else { panic!("{inclined:?}") };
    assert!((normal[0] - 0.5).abs() < 1e-6 && (normal[2] - 0.866025).abs() < 1e-5, "{normal:?}");
    let Strategy::Points { count, at } = &inclined.strategy else { panic!("{inclined:?}") };
    assert_eq!((*count, at.len()), (4, 4));
    // the points lie on the chamfer
    for q in at {
        assert!(q[0] >= 70.0 - 1e-6 && (q[2] - (40.0 - (q[0] - 70.0) * 30f64.to_radians().tan())).abs() < 1e-3, "{q:?}");
    }
    assert!(p.characteristics.iter().any(|c| matches!(&c.kind, K::Angle { nominal, .. } if (*nominal - 30.0).abs() < 1e-6)));
}

/// The export writes the neutral program beside the plan and runs the setup's post with it;
/// the post's answer comes back, and a failing post's message too.
#[cfg(unix)]
#[test]
fn the_export_runs_the_postprocessor() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = std::env::temp_dir().join(format!("sc-post-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let post = dir.join("fake-post");
    std::fs::write(
        &post,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$2/args.txt\"\nmkdir -p \"$2/plan\"\necho 'not json'\necho '{\"folder\": \"'$2'/plan\", \"skipped\": [\"Winkel\"]}'\n",
    )
    .unwrap();
    std::fs::set_permissions(&post, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut s = block();
    run(&mut s, "cmm.setup", json!({}));
    run(&mut s, "cmm.nc", json!({"post": post.display().to_string(), "plan_name": "P1", "output_dir": dir.display().to_string()}));
    run(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    run(&mut s, "cmm.cylinder", json!({"face": face(BORE_M)}));
    let r = run(&mut s, "cmm.export", json!({"replace": true}));
    assert_eq!(r["skipped"], json!(["Winkel"]));
    assert_eq!(r["folder"], format!("{}/plan", dir.display()));
    let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [dir.join("P1.cmm-program.json").display().to_string(), dir.display().to_string(), "--replace".into()]
    );
    // the program, CAD model and setup sheet lie in the plan's folder, with the plan
    let plan = dir.join("plan");
    assert_eq!(r["program"], json!(plan.join("P1.cmm-program.json").display().to_string()));
    assert!(!dir.join("P1.cmm-program.json").exists());
    let program: solidcraft_inspection::program::Program =
        serde_json::from_str(&std::fs::read_to_string(plan.join("P1.cmm-program.json")).unwrap()).unwrap();
    assert_eq!(program.plan, "P1");
    // the measured body as STEP in the base system: origin at the front left top corner (3-2-1
    // on top, front and left), so the part lies in +X, +Y and below Z 0
    let step = std::fs::read_to_string(plan.join("P1.step")).unwrap();
    assert_eq!(r["step"], json!(plan.join("P1.step").display().to_string()));
    // the vertices' points: a surface's placement (a plane's origin) may lie off the part
    let points: std::collections::HashMap<&str, [f64; 3]> = step
        .lines()
        .filter_map(|l| l.split_once("=CARTESIAN_POINT('',(").map(|(id, rest)| (id.trim(), rest)))
        .filter_map(|(id, rest)| {
            let v: Vec<f64> = rest.split(')').next()?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            Some((id, [*v.first()?, *v.get(1)?, *v.get(2)?]))
        })
        .collect();
    let pts: Vec<[f64; 3]> = step
        .lines()
        .filter_map(|l| l.split_once("=VERTEX_POINT('',").and_then(|(_, rest)| points.get(rest.split(')').next()?.trim())))
        .copied()
        .collect();
    assert!(pts.len() > 8, "{}", &step[..step.len().min(300)]);
    let min = |i: usize| pts.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min);
    let max = |i: usize| pts.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max);
    assert!(min(0).abs() < 1e-6 && min(1).abs() < 1e-6 && max(2).abs() < 1e-6, "x {} y {} z {}", min(0), min(1), max(2));
    let nc = s.doc.cmm.setups[0].programs[0].id;
    run(&mut s, "cmm.nc", json!({"nc": nc, "step": false}));
    std::fs::remove_file(plan.join("P1.step")).unwrap();
    let r = run(&mut s, "cmm.export", json!({"replace": true}));
    assert!(r["step"].is_null() && !plan.join("P1.step").exists());
    run(&mut s, "cmm.nc", json!({"nc": nc, "step": true}));
    let r = run(&mut s, "cmm.export", json!({"replace": true}));
    // the setup sheet is written beside the plan unless the NC program turns it off
    assert_eq!(r["sheet"], json!(plan.join("P1_setup-sheet.pdf").display().to_string()));
    assert!(plan.join("P1_setup-sheet.pdf").is_file());
    std::fs::remove_file(plan.join("P1_setup-sheet.pdf")).unwrap();
    assert!(s.execute("cmm.nc", &json!({"nc": nc, "sheet": "no"})).is_err());
    run(&mut s, "cmm.nc", json!({"nc": nc, "sheet": false}));
    assert!(s.doc.cmm.setups[0].programs[0].no_sheet);
    let r = run(&mut s, "cmm.export", json!({"replace": true}));
    assert!(r["sheet"].is_null() && !plan.join("P1_setup-sheet.pdf").exists(), "{r}");
    run(&mut s, "cmm.nc", json!({"nc": nc, "sheet": true}));
    assert!(!s.doc.cmm.setups[0].programs[0].no_sheet);
    std::fs::write(&post, "#!/bin/sh\necho 'Taster fehlt' >&2\nexit 3\n").unwrap();
    let e = s.execute("cmm.export", &json!({})).unwrap_err().to_string();
    assert!(e.contains("Taster fehlt"), "{e}");
    run(&mut s, "cmm.nc", json!({"nc": nc, "post": dir.join("nothing").display().to_string()}));
    assert!(s.execute("cmm.export", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Measurement results come back through the post (`--results <plan folder>`), land on the
/// model in model coordinates with their deviation from the nominal surface, and go again.
#[cfg(unix)]
#[test]
fn measurement_results_are_read_back_through_the_post() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = std::env::temp_dir().join(format!("sc-results-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("P1")).unwrap();
    let post = dir.join("fake-post");
    std::fs::write(&post, "#!/bin/sh\n[ \"$1\" = --results ] || exit 2\ncat \"$2/answer.json\"\n").unwrap();
    std::fs::set_permissions(&post, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut s = block();
    run(&mut s, "cmm.setup", json!({}));
    assert!(s.execute("cmm.results", &json!({})).is_err(), "no NC program, no plan folder");
    run(&mut s, "cmm.nc", json!({"post": post.display().to_string(), "plan_name": "P1", "output_dir": dir.display().to_string()}));
    run(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    run(&mut s, "cmm.cylinder", json!({"face": face(BORE_M)}));
    let program: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    let base = program.base_system.unwrap();
    let (name, top, diameter) = program
        .elements
        .iter()
        .find_map(|e| match e.geometry {
            solidcraft_inspection::program::Geometry::Cylinder { top, diameter, .. } => Some((e.name.clone(), top, diameter)),
            _ => None,
        })
        .unwrap();
    // one point 0.02 inside the bore wall (material too much), 2 mm below its opening, in the
    // base system as the measuring software reports it
    let surface = [top[0] + diameter / 2.0 - 0.02, top[1], top[2] - 2.0];
    let at = base.point(surface);
    let answer = json!({
        "format": "solidcraft-cmm-results/1",
        "plan": "P1",
        "measured": "2026-10-27T11:34:20",
        "elements": [{"name": name, "points": [{"at": at, "dir": base.direction([1.0, 0.0, 0.0])}]}],
        "characteristics": [
            {"name": "Durchmesser", "actual": diameter - 0.04, "nominal": diameter, "upper": 0.05, "lower": -0.05, "unit": "mm", "state": "in"},
            {"name": "Lage", "actual": 1.2, "nominal": 1.0, "upper": 0.1, "lower": -0.1, "unit": "mm", "state": "out"}
        ]
    });
    std::fs::write(dir.join("P1").join("answer.json"), serde_json::to_string(&answer).unwrap()).unwrap();
    let r = run(&mut s, "cmm.results", json!({}));
    assert_eq!((r["points"].as_u64(), r["in_tolerance"].as_u64(), r["out_of_tolerance"].as_u64()), (Some(1), Some(1), Some(1)), "{r}");
    assert_eq!(r["folder"], json!(dir.join("P1").display().to_string()));
    assert_eq!(r["characteristics"][1]["state"], "out");
    let loaded = s.cmm_results.as_ref().unwrap();
    let p = &loaded.points[0];
    assert!((0..3).all(|i| (p.at[i] - surface[i]).abs() < 1e-9), "{:?} vs {surface:?}", p.at);
    assert!((p.deviation.unwrap() - 0.02).abs() < 1e-9, "{p:?}");
    // a broken answer is refused and keeps what was shown
    std::fs::write(dir.join("P1").join("answer.json"), r#"{"format": "solidcraft-cmm-results/1", "elements": 5}"#).unwrap();
    assert!(s.execute("cmm.results", &json!({})).is_err());
    assert!(s.cmm_results.is_some());
    assert!(s.execute("cmm.results", &json!({"path": dir.join("nothing").display().to_string()})).is_err());
    assert!(s.execute("cmm.results", &json!({"clear": "yes"})).is_err());
    run(&mut s, "cmm.results", json!({"clear": true}));
    assert!(s.cmm_results.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The whole way with a real postprocessor (`SOLIDCRAFT_CMM_POST`: its program; optionally
/// `SOLIDCRAFT_CMM_PROBE_FILE`, and `SOLIDCRAFT_CMM_OUT` to keep the plan folder).
#[test]
fn local_post_exports_a_plan() {
    let Ok(post) = std::env::var("SOLIDCRAFT_CMM_POST") else { return };
    let mut s = block();
    let out = std::env::temp_dir().join(format!("sc-cmm-post-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let mut settings = json!({});
    if let Ok(probe) = std::env::var("SOLIDCRAFT_CMM_PROBE_FILE") {
        settings["probe_file"] = json!(probe);
    }
    run(&mut s, "cmm.setup", settings);
    run(&mut s, "cmm.nc", json!({"post": post, "plan_name": "SC_Klotz_Setup", "output_dir": out.display().to_string()}));
    run(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    run(&mut s, "cmm.cylinder", json!({"face": face(BORE_M), "tol": 0.02}));
    run(&mut s, "cmm.cylinder", json!({"face": face(BLIND_S1)}));
    run(&mut s, "cmm.distance", json!({"from": face(BORE_B1), "to": face(BORE_B2)}));
    run(&mut s, "cmm.distance", json!({"from": face(FRONT), "to": face(BACK)}));
    run(&mut s, "cmm.angle", json!({"first": face(TOP), "second": face(FRONT)}));
    let r = run(&mut s, "cmm.export", json!({}));
    let folder = std::path::PathBuf::from(r["folder"].as_str().unwrap());
    assert!(folder.join("inspection").is_file() && folder.join("username").is_file(), "{r}");
    assert!(s.execute("cmm.export", &json!({})).is_err(), "an existing plan is not overwritten");
    run(&mut s, "cmm.export", json!({"replace": true}));
    if let Ok(keep) = std::env::var("SOLIDCRAFT_CMM_OUT") {
        let _ = std::fs::remove_dir_all(std::path::Path::new(&keep).join("SC_Klotz_Setup"));
        let _ = std::fs::create_dir_all(&keep);
        let _ = std::fs::rename(&folder, std::path::Path::new(&keep).join("SC_Klotz_Setup"));
    }
    let _ = std::fs::remove_dir_all(&out);
}

/// A pocket's walls are only 8 mm high: their probing circles still fit, and a distance
/// between them goes into the plan (a test part of 22.10.2026).
#[test]
fn narrow_pocket_walls_get_probing_circles() {
    let mut s = block();
    run(&mut s, "solid.box", json!({"corner": [-45, 5, -8], "length": 20, "width": 20, "height": 8, "operation": "cut"}));
    run(&mut s, "cmm.setup", json!({}));
    run(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    let r = run(&mut s, "cmm.distance", json!({"from": face([-45.0, 15.0, -4.0]), "to": face([-25.0, 15.0, -4.0])}));
    assert!(r["characteristics"][0]["nominal"].as_str().is_some_and(|n| n.starts_with("20 ")), "{r}");
    let model = s.world_state().unwrap();
    let wall = crate::inspection::analyze(&model, &crate::inspection::FaceRef { body: "Klotz".into(), point: [-45.0, 15.0, -4.0] }).unwrap();
    let circles = crate::inspection::circles(&wall, 2).unwrap();
    for (c, d) in &circles {
        assert!(c[2] - d / 2.0 >= -8.0 + 1.0 - 1e-9 && c[2] + d / 2.0 <= -1.0 + 1e-9, "{c:?} Ø{d} keeps 1 mm from the wall's edges");
    }
    let st = &s.doc.cmm.setups[0];
    let program = crate::inspection::program(&model, st, &crate::inspection::default_program(st), "Tasche").unwrap();
    assert_eq!(program.characteristics.len(), 1);
}

#[test]
#[ignore]
/// Lists every face of a design (`SC_DUMP_DESIGN=path`, run with --ignored --nocapture): what the
/// CMM analysis makes of it. A tool for planning measurements, not a check.
fn dump_faces_of_a_design() {
    let Ok(path) = std::env::var("SC_DUMP_DESIGN") else { return };
    let mut s = Session::default();
    run(&mut s, "doc.open", json!({"path": path}));
    let w = s.world_state().unwrap();
    for b in &w.bodies {
        let m = b.mesh();
        let mut seen = std::collections::BTreeSet::new();
        let nfaces = m.tri_face.iter().copied().max().unwrap_or(0) as usize + 1;
        for f in 0..nfaces {
            let patch = m.surface_patch(f);
            let first = patch.iter().copied().min().unwrap_or(f);
            if !seen.insert(first) {
                continue;
            }
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            let mut sample = None;
            for (t, tf) in m.triangles.iter().zip(&m.tri_face) {
                if patch.contains(&(*tf as usize))
                    && let Some(tri) = m.tri(t)
                {
                    for p in tri {
                        for (i, c) in [p.x, p.y, p.z].into_iter().enumerate() {
                            lo[i] = lo[i].min(c);
                            hi[i] = hi[i].max(c);
                        }
                    }
                    if sample.is_none() {
                        sample = Some([
                            (tri[0].x + tri[1].x + tri[2].x) / 3.0,
                            (tri[0].y + tri[1].y + tri[2].y) / 3.0,
                            (tri[0].z + tri[1].z + tri[2].z) / 3.0,
                        ]);
                    }
                }
            }
            let surf = solidcraft_kernel::face_surface(&b.body, f);
            let r = |v: [f64; 3]| v.map(|x| (x * 1000.0).round() / 1000.0);
            let pt = sample.unwrap_or_default();
            let kind = crate::inspection::analyze(&w, &crate::inspection::FaceRef { body: b.name.clone(), point: pt })
                .map(|i| format!("{:?}", i.kind))
                .unwrap_or_else(|e| format!("({e})"));
            println!("FACE {} {first} lo {:?} hi {:?} pt {:?}\n   SURF {:?}\n   INSP {kind}", b.name, r(lo), r(hi), r(pt), surf);
        }
    }
}

/// The setup holds only the probe: postprocessor and plan output belong to its NC programs.
#[test]
fn the_setup_holds_no_post_settings() {
    let mut s = Session::default();
    let r = run(&mut s, "cmm.setup", json!({}));
    for k in ["post", "plan_name", "output_dir", "options", "sort_by_probe"] {
        let mut p = json!({"setup": r["setup"]});
        p[k] = json!(null);
        assert!(s.execute("cmm.setup", &p).is_err(), "{k}");
    }
    assert!(r["probe"].get("post").is_none() && r.get("sort_by_probe").is_none(), "{r}");
}

/// The alignment can measure its bores as circles: the origin is the circle's centre; a circle
/// characteristic on the same bore is that element (its points), not a second one.
#[test]
fn alignment_bores_can_be_circles() {
    use solidcraft_inspection::program::{Geometry, Origin, Program, Strategy};
    let mut s = block();
    run(&mut s, "cmm.setup", json!({}));
    run(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(BORE_M)], "bores": "circle"}));
    let st = run(&mut s, "cmm.state", json!({}));
    assert_eq!(st["setups"][0]["alignment"]["bores"], json!("circle"), "{st}");
    let p: Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    let Origin::Axis { element } = &p.alignment.origin_x else { panic!("{:?}", p.alignment.origin_x) };
    let e = p.elements.iter().find(|e| &e.name == element).unwrap();
    assert!(matches!(e.geometry, Geometry::Circle { .. }), "{e:?}");
    assert!(matches!(&e.strategy, Strategy::Points { count: 4, .. }));
    assert_eq!(p.alignment.origin_y, p.alignment.origin_x);

    // with a circle characteristic of 6 points on the bore: one element, 6 points
    run(&mut s, "cmm.circle", json!({"face": face(BORE_M), "points": 6, "height": 3}));
    let p: Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    let circles: Vec<_> = p.elements.iter().filter(|e| matches!(e.geometry, Geometry::Circle { .. })).collect();
    assert_eq!(circles.len(), 1, "{circles:?}");
    assert!(matches!(&circles[0].strategy, Strategy::Points { count: 6, .. }));
    let Origin::Axis { element } = &p.alignment.origin_x else { panic!() };
    assert_eq!(element, &circles[0].name);
    assert!(s.execute("cmm.alignment", &json!({"bores": "kugel"})).is_err());
    // back to the cylinder
    run(&mut s, "cmm.alignment", json!({"bores": "cylinder"}));
    let p: Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    let Origin::Axis { element } = &p.alignment.origin_x else { panic!() };
    assert!(matches!(p.elements.iter().find(|e| &e.name == element).unwrap().geometry, Geometry::Cylinder { .. }));
}

/// The alignment lists the measured elements it fixes; a measurement of one of their faces
/// is what the tree opens to change its strategy.
#[test]
fn the_alignment_lists_its_measured_elements() {
    let mut s = block();
    setup_with_alignment(&mut s);
    let st = run(&mut s, "cmm.state", json!({}));
    let els = st["setups"][0]["alignment"]["elements"].as_array().unwrap().clone();
    let kinds: Vec<&str> = els.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["plane", "cylinder", "cylinder", "cylinder"], "{els:?}");
    assert_eq!(els[3]["role"], json!("Origin X, Origin Y"));
    assert!(els.iter().all(|e| e["characteristic"].is_null()));
    assert_eq!(els[0]["strategy"], json!("Setup: 4 points"));
    // without a measurement of their own they still show where they are probed
    assert!(els.iter().all(|e| e["markers"].as_array().is_some_and(|m| !m.is_empty())), "{els:?}");
    assert_eq!(els[0]["markers"].as_array().unwrap().iter().filter(|m| m["type"] == "point").count(), 4, "{:?}", els[0]["markers"]);
    // a plane measurement of the primary plane: the alignment element points to it
    let c = run(&mut s, "cmm.plane", json!({"face": face(TOP), "strategy": "circles"}))["characteristic"].as_u64().unwrap();
    let st = run(&mut s, "cmm.state", json!({}));
    let e = &st["setups"][0]["alignment"]["elements"][0];
    assert_eq!(e["characteristic"], json!(c));
    assert_eq!(e["strategy"], json!("Circular paths"));
    // measured as circles, the bores are circles
    run(&mut s, "cmm.alignment", json!({"bores": "circle"}));
    let st = run(&mut s, "cmm.state", json!({}));
    let e = &st["setups"][0]["alignment"]["elements"][1];
    assert_eq!((e["kind"].as_str(), e["command"].as_str()), (Some("circle"), Some("cmm.circle")));
}

/// NC programs: each chooses the characteristics it outputs and its own post settings; a
/// second one gets its own plan name.
#[test]
fn nc_programs_choose_what_they_output() {
    let mut s = block();
    setup_with_alignment(&mut s);
    let add = |s: &mut Session, cmd: &str, p: Value| run(s, cmd, p)["characteristic"].as_u64().unwrap();
    let d1 = add(&mut s, "cmm.circle", json!({"face": face(BORE_B2), "tol": 0.02}));
    let dist = add(&mut s, "cmm.distance", json!({"from": face(FRONT), "to": face(BACK)}));
    // no NC program yet: the program has everything
    let all: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    assert_eq!(all.characteristics.len(), 2, "{:?}", all.characteristics);
    let r = run(&mut s, "cmm.nc", json!({"characteristics": [dist], "options": {"sort_by_probe": false, "dme_id": "M1"}, "plan_name": "Plan_A"}));
    let nc1 = r["nc"].as_u64().unwrap();
    assert_eq!(r["programs"][0]["name"], json!("NC Program 1"));
    assert_eq!(r["programs"][0]["outputs"], json!("1/2"));
    let st = &s.doc.cmm.setups[0];
    assert_eq!(st.programs[0].options.get("sort_by_probe").map(String::as_str), Some("off"));
    let p1: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({"nc": nc1}))).unwrap();
    assert_eq!(p1.plan, "Plan_A");
    assert_eq!(p1.characteristics.len(), 1);
    assert_eq!(p1.post_options.get("dme_id").map(String::as_str), Some("M1"));
    assert!(p1.characteristics.iter().all(|c| c.name.contains("Distance")), "{:?}", p1.characteristics);
    let _ = d1;
    // a second program: all characteristics, its own plan name after the setup's
    let r = run(&mut s, "cmm.nc", json!({}));
    let nc2 = r["nc"].as_u64().unwrap();
    assert_eq!(r["programs"][1]["name"], json!("NC Program 2"));
    let p2: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({"nc": nc2}))).unwrap();
    assert_eq!(p2.characteristics.len(), 2);
    assert!(p2.plan.ends_with("_NC_Program_2"), "{}", p2.plan);
    // edits keep what is not given; bad ids are refused
    run(&mut s, "cmm.nc", json!({"nc": nc1, "name": "Machine A"}));
    assert_eq!(s.doc.cmm.setups[0].programs[0].name, "Machine A");
    assert_eq!(s.doc.cmm.setups[0].programs[0].characteristics, Some(vec![dist]));
    assert!(s.execute("cmm.nc", &json!({"nc": nc1, "characteristics": [999]})).is_err());
    assert!(s.execute("cmm.nc", &json!({"nc": 4242})).is_err());
    run(&mut s, "cmm.nc.delete", json!({"nc": nc2}));
    assert_eq!(s.doc.cmm.setups[0].programs.len(), 1);
}

/// A point group: many points along a contour (here across the top and the front), each
/// checked along its surface normal, or by its coordinates.
#[test]
fn a_point_group_checks_a_contour() {
    use solidcraft_inspection::program::{CharacteristicKind as K, Geometry};
    let mut s = block();
    setup_with_alignment(&mut s);
    let pts = [[-40.0, 20.0, 0.0], [0.0, 20.0, 0.0], [40.0, 20.0, 0.0], [-20.0, -30.0, -10.0], [20.0, -30.0, -20.0]];
    let points: Vec<Value> = pts.iter().map(|p| face(*p)).collect();
    let r = run(&mut s, "cmm.point", json!({"points": points, "tol": 0.1, "name": "Kontur"}));
    let c = r["characteristics"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(c["nominal"], json!("5 points ±0.1 (normal)"), "{c}");
    assert_eq!(c["markers"].as_array().unwrap().len(), 5, "{c}");
    let p: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    let devs: Vec<&String> =
        p.characteristics.iter().filter_map(|c| if let K::Deviation { element } = &c.kind { Some(element) } else { None }).collect();
    assert_eq!(devs.len(), 5);
    assert!(devs.iter().all(|e| p.elements.iter().any(|x| &&x.name == e && matches!(x.geometry, Geometry::Point { .. }))));
    // the front points are probed along -Y, the top ones from above
    let front = p.elements.iter().find(|e| e.name == "Kontur P4").unwrap();
    assert!(matches!(front.geometry, Geometry::Point { normal, .. } if normal[1] < -0.99), "{:?}", front.geometry);
    // as coordinates: X, Y, Z for each point
    let id = c["id"].as_u64().unwrap();
    run(&mut s, "cmm.point", json!({"characteristic": id, "points": points, "check": "coordinates"}));
    let p: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    assert_eq!(p.characteristics.iter().filter(|c| matches!(c.kind, K::Coordinate { .. })).count(), 15);
    assert!(s.execute("cmm.point", &json!({"points": []})).is_err());
    assert!(s.execute("cmm.point", &json!({"points": [face([0.0, 0.0, 50.0])]})).is_err(), "a point off the part");
}

/// The setup sheet: a PDF with the cover, one picture per alignment step (two to a page) and
/// the characteristics (`SOLIDCRAFT_SHEET_OUT`: keep it there to look at).
#[test]
fn the_setup_sheet_is_a_pdf_for_the_technician() {
    let mut s = block();
    run(&mut s, "cmm.setup", json!({}));
    run(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    run(&mut s, "cmm.cylinder", json!({"face": face(BORE_M), "tol": 0.02}));
    run(&mut s, "cmm.distance", json!({"from": face(FRONT), "to": face(BACK)}));
    run(&mut s, "cmm.point", json!({"points": [face([-40.0, 20.0, 0.0]), face([40.0, 20.0, 0.0])]}));
    let out = std::env::var("SOLIDCRAFT_SHEET_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join(format!("sc-sheet-{}.pdf", std::process::id())));
    let r = run(&mut s, "cmm.sheet", json!({"path": out.display().to_string()}));
    let pdf = std::fs::read(&out).unwrap();
    assert_eq!(r["bytes"].as_u64(), Some(pdf.len() as u64));
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.starts_with("%PDF-1.4") && text.trim_end().ends_with("%%EOF"));
    // cover, steps (primary, front, left: two pages), characteristics
    assert!(text.contains("/Count 4"), "{}", &text[..text.len().min(400)]);
    // the overview and one picture per step
    assert_eq!(text.matches("/Subtype /Image").count(), 4);
    assert!(text.contains("Step 1 \\267 Primary plane"), "step title");
    // German when German is chosen: the sheet, and the names that go into the plan
    use crate::inspection_text::{Lang, with_language};
    with_language(Lang::De, || {
        run(&mut s, "cmm.sheet", json!({"path": out.display().to_string()}));
        let text = String::from_utf8_lossy(&std::fs::read(&out).unwrap()).into_owned();
        assert!(text.contains("Schritt 1 \\267 Prim\\344rebene"), "German step title");
        let p = run(&mut s, "cmm.program", json!({}));
        assert_eq!(p["alignment"]["primary"], "Ebene +Z Z0");
        assert!(p["elements"].as_array().unwrap().iter().any(|e| e["name"] == "Zylinder Ø20 X0 Y0"), "{p}");
    });
    if std::env::var("SOLIDCRAFT_SHEET_OUT").is_err() {
        let _ = std::fs::remove_file(&out);
    }
}

/// A point on a bore's wall is probed exactly radially (not along the facet of the display
/// mesh, a few degrees off), and a point on a curved face gets the surface's normal.
#[test]
fn points_on_a_bore_wall_have_the_exact_normal() {
    let mut s = block();
    run(&mut s, "cmm.setup", json!({}));
    run(&mut s, "cmm.alignment", json!({"primary": face(TOP), "direction": [face(FRONT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]}));
    let at = |deg: f64| face([10.0 * deg.to_radians().cos(), 10.0 * deg.to_radians().sin(), -10.0]);
    run(&mut s, "cmm.point", json!({"points": [at(0.0), at(30.0), at(45.0), at(200.0)], "name": "Wand"}));
    let p: solidcraft_inspection::program::Program = serde_json::from_value(run(&mut s, "cmm.program", json!({}))).unwrap();
    for (i, deg) in [0.0f64, 30.0, 45.0, 200.0].into_iter().enumerate() {
        let e = p.element(&format!("Wand P{}", i + 1)).unwrap();
        let solidcraft_inspection::program::Geometry::Point { normal, .. } = e.geometry else { panic!("{e:?}") };
        let want = [-deg.to_radians().cos(), -deg.to_radians().sin(), 0.0];
        assert!((0..3).all(|k| (normal[k] - want[k]).abs() < 1e-5), "P{}: {normal:?} != {want:?}", i + 1);
    }
}

/// Turns a point of the test block 30° about the model's X axis, as `solid.move` turns the block.
fn turned(p: [f64; 3]) -> [f64; 3] {
    let (s, c) = 30f64.to_radians().sin_cos();
    [p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c]
}

/// A part lying turned in space: the alignment's primary plane may face any way, the measuring
/// axes follow the part, not the model's axes.
#[test]
fn a_turned_part_is_aligned_on_its_own_faces() {
    use solidcraft_inspection::program::{CharacteristicKind as K, Geometry};
    let mut s = block();
    run(&mut s, "solid.move", json!({"bodies": ["Klotz"], "axis": [1, 0, 0], "angle": "30 deg"}));
    run(&mut s, "cmm.setup", json!({}));
    let st = run(
        &mut s,
        "cmm.alignment",
        json!({"primary": face(turned(TOP)), "direction": [face(turned(FRONT))], "origin_x": [face(turned(LEFT))], "origin_y": [face(turned(FRONT))]}),
    );
    assert_eq!(st["alignment"]["complete"], true, "{}", st["alignment"]["problem"]);
    run(&mut s, "cmm.distance", json!({"from": face(turned(FRONT)), "to": face(turned(BACK))}));
    run(&mut s, "cmm.cylinder", json!({"face": face(turned(BORE_M))}));
    let v = run(&mut s, "cmm.program", json!({}));
    let p: solidcraft_inspection::program::Program = serde_json::from_value(v).unwrap();
    p.validate().unwrap();
    let b = p.base_system.unwrap();
    let near = |a: [f64; 3], e: [f64; 3]| a.iter().zip(e).all(|(a, e)| (a - e).abs() < 1e-6);
    assert!(near(b.z, turned([0.0, 0.0, 1.0])), "{:?}", b.z);
    assert!(near(b.x, [1.0, 0.0, 0.0]), "{:?}", b.x);
    assert!(near(b.origin, turned([-50.0, -30.0, 0.0])), "{:?}", b.origin);
    // in the base system the part is the untouched block: its far corner and the bore's axis
    assert!(near(b.point(turned([50.0, 30.0, -30.0])), [100.0, 60.0, -30.0]));
    let bore = p.elements.iter().find(|e| matches!(e.geometry, Geometry::Cylinder { .. })).unwrap();
    assert!(matches!(bore.geometry, Geometry::Cylinder { diameter, .. } if (diameter - 20.0).abs() < 1e-6));
    assert!(
        p.characteristics
            .iter()
            .any(|c| matches!(&c.kind, K::Distance { nominal, axis: solidcraft_inspection::program::Axis::Y, .. } if (*nominal - 60.0).abs() < 1e-6)),
        "{:?}",
        p.characteristics
    );
}

/// The face the part is clamped on: Z points into the part and a virtual plane on that face (the
/// clamping plate) is probed beside the part.
#[test]
fn a_clamping_face_is_probed_on_the_plate() {
    use solidcraft_inspection::program::{Geometry, Strategy};
    const BOTTOM: [f64; 3] = [-45.0, -25.0, -30.0];
    let mut s = block();
    run(&mut s, "cmm.setup", json!({}));
    let alignment = json!({"primary": face(BOTTOM), "clamp": true, "direction": [face(FRONT)], "origin_x": [face(LEFT)], "origin_y": [face(FRONT)]});
    let st = run(&mut s, "cmm.alignment", alignment);
    assert_eq!(st["alignment"]["complete"], true, "{}", st["alignment"]["problem"]);
    let program = |s: &mut Session| -> solidcraft_inspection::program::Program { serde_json::from_value(run(s, "cmm.program", json!({}))).unwrap() };
    let p = program(&mut s);
    p.validate().unwrap();
    let b = p.base_system.unwrap();
    assert_eq!((b.origin, b.z), ([-50.0, -30.0, -30.0], [0.0, 0.0, 1.0]));
    let plate = p.element(&p.alignment.primary).unwrap();
    assert_eq!(p.elements.first().map(|e| e.name.as_str()), Some(p.alignment.primary.as_str()));
    assert!(plate.name.starts_with("Clamping plate"), "{}", plate.name);
    assert!(matches!(plate.geometry, Geometry::Plane { .. }), "{:?}", plate.geometry);
    let Strategy::Points { at, .. } = &plate.strategy else { panic!("{:?}", plate.strategy) };
    assert_eq!(at.len(), 4);
    for q in at {
        assert!((q[2] + 30.0).abs() < 1e-9, "{q:?}");
        assert!(q[0].abs() >= 60.0 - 1e-9 && q[1].abs() >= 40.0 - 1e-9, "outside the part: {q:?}");
    }
    // given points: in the alignment's X/Y; one under the part is refused
    run(&mut s, "cmm.alignment", json!({"clamp_points": [[-20, -20], [120, -20], [120, 80]]}));
    let p = program(&mut s);
    let Strategy::Points { at, .. } = &p.element(&p.alignment.primary).unwrap().strategy else { panic!() };
    assert_eq!(at.first(), Some(&[-70.0, -50.0, -30.0]));
    run(&mut s, "cmm.alignment", json!({"clamp_points": [[-20, -20], [120, -20], [50, 30]]}));
    let e = s.execute("cmm.program", &json!({})).unwrap_err().to_string();
    assert!(e.contains("under the part"), "{e}");
}
