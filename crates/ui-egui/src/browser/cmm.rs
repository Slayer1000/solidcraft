//! The CMM Plan in the browser: the plan folder, one row per setup (the active one marked),
//! and under each setup its Probe System, Alignment (a folder with the measured elements it
//! fixes), Inspection Plan with the characteristics and NC Programs (one output each, through a
//! postprocessor). Double-clicking a row opens its dialog; the menus add, edit, delete and
//! export.

use super::*;

/// The icon of a characteristic kind (by its command).
fn kind_icon(command: &str) -> &'static str {
    match command {
        "cmm.circle" => "circle3",
        "cmm.cylinder" => "cylinder",
        "cmm.plane" => "plane",
        "cmm.distance" => "measure",
        "cmm.angle" => "plane_angle",
        "cmm.point" => "point",
        _ => "inspect",
    }
}

const ADD: [(&str, &str); 6] = [
    ("cmm.circle", "Circle"),
    ("cmm.cylinder", "Cylinder Measurement"),
    ("cmm.plane", "Plane Measurement"),
    ("cmm.distance", "Distance"),
    ("cmm.angle", "Angle"),
    ("cmm.point", "Points"),
];

fn target(what: &str, setup: Option<u64>, characteristic: Option<u64>) -> Target {
    Target::Cmm { what: what.into(), setup, characteristic }
}

pub(super) fn rows(app: &mut SolveApp, ui: &mut egui::Ui, depth: usize, acts: &mut Actions) {
    // In Design the folder shows once there is a plan; the CMM workspace always shows it.
    if app.session.doc.cmm.is_empty() && !crate::cmm_workspace::active(app) {
        return;
    }
    let summary = crate::cmm_workspace::summary(app);
    let key = "cmm";
    let open = is_open(app, key, true);
    let r = draw_row(ui, ui.scope_id().with(key), &Row { depth, fold: Some(open), icon: "folder", label: "CMM Plan", ..Default::default() });
    if r.fold || r.clicked {
        toggle(app, key, true);
    }
    if r.double {
        app.start("cmm.setup");
    }
    if let Some(p) = r.secondary {
        acts.menu = Some((p, target("plan", None, None)));
    }
    if !open {
        return;
    }
    let setups = summary["setups"].as_array().cloned().unwrap_or_default();
    if setups.is_empty() {
        let r = draw_row(
            ui,
            ui.scope_id().with("cmm_empty"),
            &Row { depth: depth + 1, icon: "new", label: "New Setup", dim: true, ..Default::default() },
        );
        if r.clicked || r.double {
            app.start("cmm.setup");
        }
    }
    let t = Tokens::get();
    for st in setups {
        let Some(id) = st["id"].as_u64() else { continue };
        let name = st["name"].as_str().unwrap_or("Setup").to_string();
        let active = st["active"] == true;
        let skey = format!("cmm:{id}");
        let sopen = is_open(app, &skey, true);
        let r = draw_row(
            ui,
            ui.scope_id().with(&skey),
            &Row { depth: depth + 1, fold: Some(sopen), radio: Some(active), icon: "folder", label: &name, name: true, ..Default::default() },
        );
        if r.fold || r.clicked {
            toggle(app, &skey, true);
        }
        if r.radio && !active {
            let _ = app.run("cmm.setup.activate", json!({"setup": id}));
        }
        if r.double {
            crate::dialogs_cmm::edit(app, id, "setup", None);
        }
        if let Some(p) = r.secondary {
            acts.menu = Some((p, target("setup", Some(id), None)));
        }
        if !sopen {
            continue;
        }
        // Probe System
        let probe = st["probe"]["probe_file"].as_str().map_or_else(
            || crate::i18n::tr_label("from the post").to_string(),
            |p| std::path::Path::new(p).file_stem().and_then(|s| s.to_str()).unwrap_or(p).to_string(),
        );
        let g = &st["probe"]["probe_geometry"];
        let dims = match (g["ball"].as_f64(), g["length"].as_f64()) {
            (Some(b), Some(l)) => format!(" · Ø{} L{}", solidcraft_engine::inspection::num(b), solidcraft_engine::inspection::num(l)),
            _ => String::new(),
        };
        let safety = st["safety_plane"].as_str().map(|s| format!(" · SP {}", s.to_uppercase())).unwrap_or_default();
        let label = format!("{}: {probe}{dims}{safety}", crate::i18n::tr_label("Probe System"));
        let r = draw_row(
            ui,
            ui.scope_id().with((&skey, "probe")),
            &Row { depth: depth + 2, icon: "settings", label: &label, name: true, ..Default::default() },
        );
        if r.double {
            crate::dialogs_cmm::edit(app, id, "setup", None);
        }
        if let Some(p) = r.secondary {
            acts.menu = Some((p, target("probe", Some(id), None)));
        }
        // Alignment
        let complete = st["alignment"]["complete"] == true;
        let label = if complete {
            crate::i18n::tr_label("Alignment").to_string()
        } else {
            format!("{} ({})", crate::i18n::tr_label("Alignment"), crate::i18n::tr_label("incomplete"))
        };
        let elements = st["alignment"]["elements"].as_array().cloned().unwrap_or_default();
        let akey = format!("cmm:{id}:alignment");
        let aopen = is_open(app, &akey, true);
        let r = draw_row(
            ui,
            ui.scope_id().with((&skey, "alignment")),
            &Row {
                depth: depth + 2,
                fold: (!elements.is_empty()).then_some(aopen),
                icon: "origin",
                label: &label,
                name: true,
                dim: !complete,
                ..Default::default()
            },
        );
        if let Some(rect) = r.rect
            && let Some(problem) = st["alignment"]["problem"].as_str()
        {
            ui.interact(rect, ui.scope_id().with((&skey, "alignment_tip")), Sense::hover()).on_hover_text(problem);
        }
        if r.fold {
            toggle(app, &akey, true);
        }
        if r.double || (r.clicked && !complete) {
            crate::dialogs_cmm::edit(app, id, "alignment", None);
        }
        if let Some(p) = r.secondary {
            acts.menu = Some((p, target("alignment", Some(id), None)));
        }
        // the measured elements the alignment fixes: double-click opens their measurement
        if aopen {
            for (i, e) in elements.iter().enumerate() {
                let label = format!(
                    "{}: {} · {}",
                    e["role"].as_str().unwrap_or_default(),
                    e["name"].as_str().unwrap_or_default(),
                    e["strategy"].as_str().unwrap_or_default()
                );
                let command = e["command"].as_str().unwrap_or_default();
                let r = draw_row(
                    ui,
                    ui.scope_id().with((&akey, i)),
                    &Row { depth: depth + 3, icon: kind_icon(command), label: &label, name: true, ..Default::default() },
                );
                if let Some(rect) = r.rect {
                    let tip = if e["characteristic"].is_u64() {
                        crate::i18n::tr_label("Measured as its measurement in the inspection plan says (double-click to change)")
                    } else {
                        crate::i18n::tr_label("Measured with the default strategy (double-click to set your own)")
                    };
                    ui.interact(rect, ui.scope_id().with((&akey, i, "tip")), Sense::hover()).on_hover_text(tip);
                }
                if r.clicked {
                    select_faces(app, &json!([e["face"]]));
                    app.cmm.focus = None;
                    app.cmm.focus_alignment = Some((id, i));
                }
                if r.double {
                    match e["characteristic"].as_u64() {
                        Some(cid) => crate::dialogs_cmm::edit(app, id, "characteristic", Some(cid)),
                        None => {
                            if let Ok(face) = serde_json::from_value(e["face"].clone()) {
                                crate::dialogs_cmm::measure_face(app, id, command, &face);
                            }
                        }
                    }
                }
                if let Some(p) = r.secondary {
                    acts.menu = Some((p, target("alignment", Some(id), None)));
                }
            }
        }
        // Inspection Plan
        let pkey = format!("cmm:{id}:plan");
        let popen = is_open(app, &pkey, true);
        let chars = st["characteristics"].as_array().cloned().unwrap_or_default();
        let r = draw_row(
            ui,
            ui.scope_id().with(&pkey),
            &Row { depth: depth + 2, fold: Some(popen), icon: "folder", label: "Inspection Plan", ..Default::default() },
        );
        if r.fold || r.clicked {
            toggle(app, &pkey, true);
        }
        if let Some(p) = r.secondary {
            acts.menu = Some((p, target("characteristics", Some(id), None)));
        }
        for c in chars.into_iter().filter(|_| popen) {
            let Some(cid) = c["id"].as_u64() else { continue };
            let name = c["name"].as_str().unwrap_or_default();
            let probe = c["probe_file"]
                .as_str()
                .map(|p| format!(" · {}", std::path::Path::new(p).file_stem().and_then(|s| s.to_str()).unwrap_or(p)))
                .unwrap_or_default();
            let label = match c["nominal"].as_str() {
                Some(n) => format!("{name} · {n}{probe}"),
                None => format!("{name} · ⚠ {}", c["error"].as_str().unwrap_or_default()),
            };
            let error = c["error"].is_string();
            let r = draw_row(
                ui,
                ui.scope_id().with(("cmm_char", cid)),
                &Row {
                    depth: depth + 3,
                    icon: kind_icon(c["command"].as_str().unwrap_or_default()),
                    label: &label,
                    name: true,
                    color: error.then_some(t.form_error),
                    ..Default::default()
                },
            );
            if let Some(rect) = r.rect {
                let kind = crate::i18n::tr_label(c["kind"].as_str().unwrap_or_default()).to_string();
                ui.interact(rect, ui.scope_id().with(("cmm_char_tip", cid)), Sense::hover()).on_hover_text(kind);
            }
            if r.clicked {
                select_faces(app, &c["faces"]);
                app.cmm.focus = Some(cid);
                app.cmm.focus_alignment = None;
            }
            if r.double {
                crate::dialogs_cmm::edit(app, id, "characteristic", Some(cid));
            }
            if let Some(p) = r.secondary {
                acts.menu = Some((p, target("characteristic", Some(id), Some(cid))));
            }
        }
        // the measurement results loaded for this setup: one row per characteristic
        result_rows(app, ui, depth + 2, id, acts);
        // NC programs: shown once the setup has one (Create Inspection Plan makes the first)
        let programs = st["programs"].as_array().cloned().unwrap_or_default();
        if programs.is_empty() {
            continue;
        }
        let nkey = format!("cmm:{id}:nc");
        let nopen = is_open(app, &nkey, true);
        let r = draw_row(
            ui,
            ui.scope_id().with(&nkey),
            &Row { depth: depth + 2, fold: Some(nopen), icon: "folder", label: "NC Programs", ..Default::default() },
        );
        if r.fold || r.clicked {
            toggle(app, &nkey, true);
        }
        if let Some(p) = r.secondary {
            acts.menu = Some((p, target("programs", Some(id), None)));
        }
        for nc in programs.iter().filter(|_| nopen) {
            let Some(nid) = nc["id"].as_u64() else { continue };
            let post = nc["post_used"].as_str().unwrap_or("–");
            let label = format!(
                "{} · {post} · {} {}",
                nc["name"].as_str().unwrap_or_default(),
                nc["outputs"].as_str().unwrap_or_default(),
                crate::i18n::tr_label("characteristics")
            );
            let r = draw_row(
                ui,
                ui.scope_id().with(("cmm_nc", nid)),
                &Row { depth: depth + 3, icon: "export", label: &label, name: true, ..Default::default() },
            );
            if let Some(rect) = r.rect {
                let tip = format!("{} → {}", nc["plan"].as_str().unwrap_or_default(), nc["output_used"].as_str().unwrap_or_default());
                ui.interact(rect, ui.scope_id().with(("cmm_nc_tip", nid)), Sense::hover()).on_hover_text(tip);
            }
            if r.double {
                crate::dialogs_cmm::edit_nc(app, id, Some(nid));
            }
            if let Some(p) = r.secondary {
                acts.menu = Some((p, target("program", Some(id), Some(nid))));
            }
        }
    }
}

/// The loaded measurement results of setup `id`: a folder with how many characteristics are in
/// and out of tolerance, and per characteristic its actual value and deviation (red: out).
fn result_rows(app: &mut SolveApp, ui: &mut egui::Ui, depth: usize, id: u64, acts: &mut Actions) {
    let Some(r) = app.session.cmm_results.as_ref().filter(|r| r.setup == id) else { return };
    let t = crate::theme::Tokens::get();
    let (inside, out) = r.counts();
    let mut chars: Vec<solidcraft_engine::inspection_results::CharacteristicResult> = r.characteristics.iter().take(5000).cloned().collect();
    chars.sort_by_cached_key(|c| natural(&c.name));
    let nc = r.nc;
    let when = r.measured.as_deref().map(|m| format!(" · {}", m.replace('T', " "))).unwrap_or_default();
    let label = format!(
        "{}{when} · {inside} {} · {out} {}",
        crate::i18n::tr_label("Measurement Results"),
        crate::i18n::tr_label("in tolerance"),
        crate::i18n::tr_label("out of tolerance")
    );
    let key = format!("cmm:{id}:results");
    let open = is_open(app, &key, true);
    let r = draw_row(
        ui,
        ui.scope_id().with(&key),
        &Row { depth, fold: Some(open), icon: "import", label: &label, name: true, color: (out > 0).then_some(t.form_error), ..Default::default() },
    );
    if r.fold || r.clicked {
        toggle(app, &key, true);
    }
    if let Some(p) = r.secondary {
        acts.menu = Some((p, target("results", Some(id), nc)));
    }
    if !open {
        return;
    }
    for c in chars {
        use solidcraft_engine::inspection_results::State;
        let unit = if c.unit == "deg" { "°" } else { " mm" };
        let delta = if c.nominal.abs() > 1e-12 { format!(" (Δ {:+.4})", c.actual - c.nominal) } else { String::new() };
        let label = format!("{:.4}{unit}{delta} · {}", c.actual, c.name);
        let focused = app.cmm.focus_result.as_deref() == Some(c.name.as_str());
        let r = draw_row(
            ui,
            ui.scope_id().with(("cmm_result", id, &c.name)),
            &Row {
                depth: depth + 1,
                icon: "point",
                label: &label,
                name: true,
                color: (c.state == State::Out).then_some(t.form_error),
                selected: focused,
                ..Default::default()
            },
        );
        if let Some(rect) = r.rect {
            let state = match c.state {
                State::In => crate::i18n::tr_label("in tolerance"),
                State::Out => crate::i18n::tr_label("out of tolerance"),
                State::Unknown => crate::i18n::tr_label("no tolerance"),
            };
            let tip = format!(
                "{}\n{:.4}{unit} · {} {:.4} · +{:.4} / {:.4}\n{state}",
                c.name,
                c.actual,
                crate::i18n::tr_label("Nominal"),
                c.nominal,
                c.upper,
                c.lower
            );
            ui.interact(rect, ui.scope_id().with(("cmm_result_tip", id, &c.name)), Sense::hover()).on_hover_text(tip);
        }
        if r.clicked {
            app.cmm.focus_result = if focused { None } else { Some(c.name.clone()) };
        }
        if let Some(p) = r.secondary {
            acts.menu = Some((p, target("results", Some(id), nc)));
        }
    }
}

/// A name for sorting with its numbers by value ("P2" before "P10").
fn natural(name: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut digits = String::new();
    for ch in name.chars().take(500) {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            if !digits.is_empty() {
                out.push((std::mem::take(&mut text), digits.parse().unwrap_or(u64::MAX)));
                digits.clear();
            }
            text.push(ch);
        }
    }
    out.push((text, digits.parse().unwrap_or(0)));
    out
}

/// Selects a characteristic's faces in the model (so it can be seen).
fn select_faces(app: &mut SolveApp, faces: &Value) {
    let items: Vec<Value> = faces
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let p = f.get("point")?;
            let at = solidcore_record::Vec3::new(p.get(0)?.as_f64()?, p.get(1)?.as_f64()?, p.get(2)?.as_f64()?);
            crate::dialogs::face_sel(&app.session, at).and_then(|s| serde_json::to_value(s).ok())
        })
        .collect();
    if !items.is_empty() {
        let _ = app.run("select.set", json!({ "items": items }));
    }
}

/// The menus of the CMM rows.
pub(crate) fn cmm_items(app: &SolveApp, what: &str, setup: Option<u64>, characteristic: Option<u64>) -> Vec<Item> {
    let add =
        || -> Vec<Item> { ADD.iter().map(|(cmd, label)| Item::action("ui.cmmAdd", label, kind_icon(cmd)).with(json!({ "command": cmd }))).collect() };
    match (what, setup) {
        ("plan", _) => vec![Item::action("cmm.setup", "New Setup", "new")],
        ("characteristic", Some(s)) => vec![
            Item::action("ui.cmmEdit", "Edit", "edit").with(json!({ "setup": s, "characteristic": characteristic })),
            Item::sep(),
            Item::action("cmm.characteristic.delete", "Delete", "delete").with(json!({ "characteristic": characteristic })),
        ],
        ("program", Some(s)) => vec![
            Item::action("ui.cmmEditNc", "Edit", "edit").with(json!({ "setup": s, "nc": characteristic })),
            Item::action("ui.cmmEditNc", "Create Inspection Plan", "export").with(json!({ "setup": s, "nc": characteristic })),
            Item::action("ui.cmmResults", "Load Measurement Results", "import").with(json!({ "setup": s, "nc": characteristic })),
            Item::sep(),
            Item::action("cmm.nc.delete", "Delete", "delete").with(json!({ "nc": characteristic })),
        ],
        ("results", Some(s)) => vec![
            Item::action("ui.cmmResults", "Reload Measurement Results", "import").with(json!({ "setup": s, "nc": characteristic })),
            Item::action("cmm.results", "Hide Measurement Results", "eye").with(json!({ "clear": true })),
        ],
        ("programs", Some(s)) => vec![Item::action("ui.cmmNewNc", "New NC Program", "new").with(json!({ "setup": s }))],
        ("characteristics", Some(s)) => {
            let mut v = add();
            v.push(Item::sep());
            v.push(Item::action("ui.cmmExport", "Create Inspection Plan", "export").with(json!({ "setup": s })));
            v
        }
        (_, Some(s)) => {
            let active = app.session.doc.cmm.active_setup().is_some_and(|a| a.id == s);
            let mut v = vec![
                Item::action("cmm.setup.activate", "Activate", "activate").with(json!({ "setup": s })).on_if(!active),
                Item::action("ui.cmmEditSetup", "Probe System…", "settings").with(json!({ "setup": s })),
                Item::action("ui.cmmEditAlignment", "Alignment…", "origin").with(json!({ "setup": s })),
                Item::action("cmm.probe.table", "Probe Table…", "configuration_table"),
                Item::sep(),
            ];
            v.extend(add());
            v.push(Item::sep());
            v.push(Item::action("ui.cmmExport", "Create Inspection Plan", "export").with(json!({ "setup": s })));
            v.push(Item::action("ui.cmmNewNc", "New NC Program", "new").with(json!({ "setup": s })));
            v.push(Item::action("cmm.setup.delete", "Delete Setup", "delete").with(json!({ "setup": s })));
            v
        }
        _ => Vec::new(),
    }
}

/// Runs a CMM menu action (`ui.cmm*`).
pub(crate) fn cmm_action(app: &mut SolveApp, id: &str, p: &Value) {
    let setup = p.get("setup").and_then(Value::as_u64);
    match (id, setup) {
        ("ui.cmmAdd", _) => {
            if let Some(cmd) = p.get("command").and_then(Value::as_str) {
                if let Some(s) = setup
                    && app.session.doc.cmm.active != Some(s)
                {
                    let _ = app.run("cmm.setup.activate", json!({ "setup": s }));
                }
                app.start(cmd);
            }
        }
        ("ui.cmmEdit", Some(s)) => crate::dialogs_cmm::edit(app, s, "characteristic", p.get("characteristic").and_then(Value::as_u64)),
        ("ui.cmmEditSetup", Some(s)) => crate::dialogs_cmm::edit(app, s, "setup", None),
        ("ui.cmmEditAlignment", Some(s)) => crate::dialogs_cmm::edit(app, s, "alignment", None),
        ("ui.cmmExport", _) => crate::cmm_workspace::export(app, setup),
        ("ui.cmmEditNc", Some(s)) => crate::dialogs_cmm::edit_nc(app, s, p.get("nc").and_then(Value::as_u64)),
        ("ui.cmmNewNc", Some(s)) => crate::dialogs_cmm::edit_nc(app, s, None),
        ("ui.cmmResults", _) => crate::cmm_workspace::load_results(app, setup, p.get("nc").and_then(Value::as_u64)),
        _ => {}
    }
}
