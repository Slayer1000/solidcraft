use crate::test_support::run_all;
use serde_json::json;
use solidcraft_engine::Session;
use solidcraft_engine::geom::Vec3;

use super::*;
use crate::Services;
use crate::dialogs::apply_commands;

/// A 100 × 60 × 30 block below Z 0 with a Ø20 bore in the middle and two Ø10 bores in front.
pub(crate) fn block_app() -> SolveApp {
    let mut s = Session::default();
    let mut run = |id: &str, p: Value| {
        s.execute(id, &p).unwrap();
    };
    run("sketch.create", json!({"plane": {"origin": [0, 0, -30], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}, "name": "Grund"}));
    run("sketch.rectangle.center", json!({"center": [0, 0], "corner": [50, 30]}));
    run("sketch.finish", json!({}));
    run("solid.extrude", json!({"distance": 30, "body_name": "Klotz"}));
    run("sketch.create", json!({"plane": {"origin": [0, 0, 0], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}, "name": "Durch"}));
    for (x, y, d) in [(0, 0, 20), (-35, -15, 10), (35, -15, 10)] {
        run("sketch.circle.center", json!({"center": [x, y], "diameter": d}));
    }
    run("sketch.finish", json!({}));
    run("solid.extrude", json!({"through_all": true, "direction": "negative", "operation": "cut", "profiles": "all"}));
    SolveApp::new(s, Services::default())
}

pub(crate) const TOP: [f64; 3] = [-45.0, -25.0, 0.0];
const FRONT: [f64; 3] = [0.0, -30.0, -15.0];
const BACK: [f64; 3] = [0.0, 30.0, -15.0];
const BORE_M: [f64; 3] = [-10.0, 0.0, -10.0];
const BORE_B1: [f64; 3] = [-40.0, -15.0, -10.0];
const BORE_B2: [f64; 3] = [30.0, -15.0, -10.0];

fn pick(app: &SolveApp, p: [f64; 3]) -> Sel {
    crate::dialogs::face_sel(&app.session, Vec3::new(p[0], p[1], p[2])).unwrap()
}

/// New Setup, then the alignment and a characteristic picked in their dialogs; the tree's edit
/// opens them again filled, and OK changes them in place.
#[test]
fn setup_alignment_and_characteristic_dialogs() {
    let mut app = block_app();
    app.start("cmm.setup");
    let d = app.dialog.clone().unwrap();
    assert!(matches!(&d.kind, Kind::Cmm(Km::Setup { name, .. }) if name == "Setup 1"));
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].0, "cmm.setup");
    run_all(&mut app, c);
    app.dialog = None;
    let id = app.session.doc.cmm.setups[0].id;

    app.start("cmm.alignment");
    let mut d = app.dialog.clone().unwrap();
    assert_eq!(d.inputs.len(), 4);
    assert!(apply_commands(&app, &d).is_err(), "nothing picked yet");
    d.inputs[0].items = vec![pick(&app, TOP)];
    d.inputs[1].items = vec![pick(&app, BORE_B1), pick(&app, BORE_B2)];
    d.inputs[2].items = vec![pick(&app, BORE_M)];
    // the coordinate system the picks make, before OK
    let a = alignment_of(&app, &d.inputs);
    assert!(a.is_complete(), "a bore for X alone is Y too");
    let f = insp::frame(&app.session.world_state(), &a).unwrap();
    assert_eq!(f.origin, [0.0, 0.0, 0.0]);
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].1["setup"], id);
    run_all(&mut app, c);
    app.dialog = None;
    assert!(app.session.doc.cmm.setups[0].alignment.is_complete());

    edit(&mut app, id, "alignment", None);
    let d = app.dialog.clone().unwrap();
    assert_eq!(d.inputs.iter().map(|i| i.items.len()).collect::<Vec<_>>(), vec![1, 2, 1, 0]);
    app.dialog = None;

    app.start("cmm.distance");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![pick(&app, FRONT)];
    d.inputs[1].items = vec![pick(&app, BACK)];
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!((c[0].0.as_str(), &c[0].1["tol"]), ("cmm.distance", &json!(0.05)));
    run_all(&mut app, c);
    app.dialog = None;
    let cid = app.session.doc.cmm.setups[0].characteristics[0].id;

    edit(&mut app, id, "characteristic", Some(cid));
    let mut d = app.dialog.clone().unwrap();
    assert_eq!(d.extra.get("characteristic"), Some(&json!(cid)));
    if let Kind::Cmm(Km::Distance { tol, axis, .. }) = &mut d.kind {
        *tol = "0,1".into();
        *axis = 2;
    }
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].1["axis"], "y");
    run_all(&mut app, c);
    let ch = &app.session.doc.cmm.setups[0].characteristics;
    assert_eq!((ch.len(), ch[0].tol), (1, 0.1), "edited in place");

    // the tree's menus
    let items = crate::browser::cmm_items(&app, "characteristics", Some(id), None);
    assert_eq!(items.iter().filter(|i| i.id == "ui.cmmAdd").count(), 6);
    let items = crate::browser::cmm_items(&app, "characteristic", Some(id), Some(cid));
    assert!(items.iter().any(|i| i.id == "cmm.characteristic.delete"));
}

/// A characteristic dialog says what is wrong with the picks instead of running the command.
#[test]
fn characteristic_dialogs_check_their_picks() {
    let mut app = block_app();
    app.run("cmm.setup", json!({})).unwrap();
    app.start("cmm.circle");
    let mut d = app.dialog.clone().unwrap();
    assert!(apply_commands(&app, &d).unwrap_err().contains("bore"));
    d.inputs[0].items = vec![pick(&app, BORE_M)];
    if let Kind::Cmm(Km::Circle { points, .. }) = &mut d.kind {
        *points = "2".into();
    }
    assert!(apply_commands(&app, &d).is_err(), "at least 3 points");
    if let Kind::Cmm(Km::Circle { points, tol, .. }) = &mut d.kind {
        *points = "4".into();
        *tol = "x".into();
    }
    assert!(apply_commands(&app, &d).is_err(), "the tolerance is a number");
    let Kind::Cmm(k) = &d.kind else { panic!("{:?}", d.kind) };
    let ch = characteristic_of(k, &d.inputs).unwrap();
    let info = insp::characteristic_info(&app.session.world_state(), None, &ch);
    assert_eq!(info["value"], 20.0);
}

/// A plane dialog with the strategy points takes the points clicked on the face (at least 4);
/// the cylinder dialog gets its points input only with that strategy; edit fills them again.
#[test]
fn strategy_points_are_picked_on_the_face() {
    let mut app = block_app();
    app.run("cmm.setup", json!({})).unwrap();
    app.start("cmm.plane");
    let mut d = app.dialog.clone().unwrap();
    assert_eq!(d.inputs.len(), 2, "plane and its points");
    assert!(d.inputs[1].places && d.inputs[1].multi);
    d.inputs[0].items = vec![pick(&app, TOP)];
    // no points picked: spread over the face
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!((c[0].1["strategy"].as_str(), c[0].1["points"].as_u64()), (Some("points"), Some(4)));
    d.inputs[1].items = [[-45.0, -25.0, 0.0], [45.0, -25.0, 0.0], [45.0, 25.0, 0.0]].iter().map(|p| pick(&app, *p)).collect();
    assert!(apply_commands(&app, &d).unwrap_err().contains("at least 4"));
    d.inputs[1].items.push(pick(&app, [-45.0, 25.0, 0.0]));
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].1["picks"].as_array().map(Vec::len), Some(4));
    let Kind::Cmm(k) = &d.kind else { panic!() };
    let ch = characteristic_of(k, &d.inputs).unwrap();
    let st = app.session.doc.cmm.setups[0].clone();
    assert_eq!(insp::characteristic_markers(&app.session.world_state(), &st, &ch).as_array().map(Vec::len), Some(4));
    run_all(&mut app, c);
    app.dialog = None;
    let cid = app.session.doc.cmm.setups[0].characteristics[0].id;
    edit(&mut app, st.id, "characteristic", Some(cid));
    let d = app.dialog.clone().unwrap();
    assert_eq!(d.inputs.iter().map(|i| i.items.len()).collect::<Vec<_>>(), vec![1, 4]);
    app.dialog = None;

    // the cylinder: points only with that strategy
    app.start("cmm.cylinder");
    let mut d = app.dialog.clone().unwrap();
    assert_eq!(d.inputs.len(), 1);
    d.inputs[0].items = vec![pick(&app, BORE_M)];
    if let Kind::Cmm(k @ Km::Cylinder { .. }) = &mut d.kind {
        if let Km::Cylinder { strategy, .. } = k {
            *strategy = 2;
        }
        let mut active = 0;
        sync_points(k, &mut d.inputs, &mut active);
        assert_eq!((d.inputs.len(), active), (2, 1), "the next clicks place points");
        if let Km::Cylinder { strategy, .. } = k {
            *strategy = 1;
        }
        sync_points(k, &mut d.inputs, &mut active);
        assert_eq!((d.inputs.len(), active), (1, 0));
    }
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!((c[0].1["strategy"].as_str(), c[0].1["points"].as_u64()), (Some("circle_points"), Some(7)));
    assert_eq!(c[0].1["safety_plane"], Value::Null, "the setup's");
}
