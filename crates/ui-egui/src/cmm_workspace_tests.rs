use super::*;
use serde_json::json;

/// The workspace switches from Design and back, shows only its tab, and its plan summary
/// follows the design.
#[test]
fn the_cmm_workspace_has_its_own_tab_and_tree() {
    let mut app = crate::dialogs_cmm::tests::block_app();
    assert_eq!(current(&app), "design");
    switch(&mut app, "cmm").unwrap();
    assert_eq!((current(&app), crate::workspace::visible_tabs(&app)), ("cmm", vec![TAB]));
    assert!(summary(&mut app)["setups"].as_array().is_some_and(Vec::is_empty));
    app.run("cmm.setup", json!({})).unwrap();
    app.run(
        "cmm.alignment",
        json!({"primary": {"body": "Klotz", "point": crate::dialogs_cmm::tests::TOP}, "direction": [{"body": "Klotz", "point": [0, -30, -15]}],
               "origin_x": [{"body": "Klotz", "point": [-50, 0, -15]}], "origin_y": [{"body": "Klotz", "point": [0, -30, -15]}]}),
    )
    .unwrap();
    let s = summary(&mut app);
    assert_eq!(s["setups"][0]["alignment"]["frame"]["origin"], json!([-50.0, -30.0, 0.0]));
    assert!(shown_frame(&mut app).is_some_and(|(f, name, preview)| f.origin == [-50.0, -30.0, 0.0] && name == "Setup 1" && !preview));
    // the commands are on the CMM tab
    let specs = solidcraft_engine::command_specs();
    let (cmds, _) = crate::workspace::layout(TAB, "CHARACTERISTICS", &specs).unwrap();
    assert_eq!(cmds.iter().map(|c| c.id).collect::<Vec<_>>(), ["cmm.plane", "cmm.cylinder", "cmm.circle", "cmm.point"]);
    let (cmds, _) = crate::workspace::layout(TAB, "RELATIONS", &specs).unwrap();
    assert_eq!(cmds.iter().map(|c| c.id).collect::<Vec<_>>(), ["cmm.distance", "cmm.angle"]);
    switch(&mut app, "render").unwrap();
    assert_eq!(current(&app), "render");
    switch(&mut app, "cmm").unwrap();
    switch(&mut app, "design").unwrap();
    assert_eq!(current(&app), "design");
    assert!(switch(&mut app, "drawing").is_err());
    // a sketch in progress keeps Design
    app.run("sketch.create", json!({"plane": "XY"})).unwrap();
    assert!(switch(&mut app, "cmm").is_err());
}
