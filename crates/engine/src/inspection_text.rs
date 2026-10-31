//! The language of the CMM workspace's texts: the names SolidCraft gives measured elements and
//! characteristics, the descriptions in the tree, messages and the setup sheet (PDF). These leave
//! the app (in the measuring software's plan, on the operator's sheet), so they are written in
//! one language: English, or German when that is the user's language.
//!
//! The source text is English; [`tr!`] looks it up in the German table when German is chosen.
//! The desktop app follows its interface language ([`set_language`]); without one (scripts, the
//! CLI) the system's (`LC_ALL`, `LC_MESSAGES`, `LANG`) decides.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// The languages of the CMM texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
}

impl Lang {
    /// German for `de`, `de-AT`, `de_DE.UTF-8`, …; English for everything else.
    pub fn from_id(id: &str) -> Lang {
        let id = id.trim().to_ascii_lowercase();
        if id == "de" || id.starts_with("de-") || id.starts_with("de_") || id.starts_with("de.") { Lang::De } else { Lang::En }
    }
    pub fn id(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
        }
    }
}

/// 0: not chosen (the system's), 1: English, 2: German.
static CHOSEN: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static SCOPED: Cell<Option<Lang>> = const { Cell::new(None) };
}

fn system() -> Lang {
    static SYSTEM: OnceLock<Lang> = OnceLock::new();
    *SYSTEM.get_or_init(|| {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.trim().is_empty())
            .map_or(Lang::En, |v| Lang::from_id(&v))
    })
}

/// Chooses the language of the CMM texts (`de`, `en`, any other id: English); an empty id
/// goes back to the system's.
pub fn set_language(id: &str) {
    let v = if id.trim().is_empty() {
        0
    } else {
        match Lang::from_id(id) {
            Lang::En => 1,
            Lang::De => 2,
        }
    };
    CHOSEN.store(v, Ordering::Relaxed);
}

/// The language of the CMM texts now (on this thread).
pub fn language() -> Lang {
    if let Some(l) = SCOPED.with(Cell::get) {
        return l;
    }
    match CHOSEN.load(Ordering::Relaxed) {
        1 => Lang::En,
        2 => Lang::De,
        // tests do not depend on the machine's language
        _ if cfg!(test) => Lang::En,
        _ => system(),
    }
}

/// Runs `f` with the CMM texts in `lang` on this thread (a setup sheet in another language,
/// tests).
pub fn with_language<T>(lang: Lang, f: impl FnOnce() -> T) -> T {
    let before = SCOPED.with(|s| s.replace(Some(lang)));
    let out = f();
    SCOPED.with(|s| s.set(before));
    out
}

fn german() -> &'static BTreeMap<&'static str, &'static str> {
    static MAP: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| GERMAN.iter().copied().collect())
}

/// `en` in the current language (English when there is no translation).
pub fn t(en: &'static str) -> &'static str {
    match language() {
        Lang::En => en,
        Lang::De => german().get(en).copied().unwrap_or(en),
    }
}

/// [`t`] with `{name}` placeholders filled in (values are taken literally).
pub fn tf(en: &'static str, args: &[(&str, String)]) -> String {
    let template = t(en);
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        out.push_str(before);
        match after.split_once('}') {
            Some((name, tail)) => {
                match args.iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = tail;
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A CMM text in the current language: `tr!("Plane")`, `tr!("{n} points", n = count)`.
#[macro_export]
macro_rules! tr {
    ($en:literal) => {
        $crate::inspection_text::t($en)
    };
    ($en:literal, $($k:ident = $v:expr),+ $(,)?) => {
        $crate::inspection_text::tf($en, &[$((stringify!($k), ($v).to_string())),+])
    };
}

/// English → German. Every text of the CMM workspace that `tr!` takes has its line here (a test
/// checks it); placeholders `{name}` stay as they are.
static GERMAN: &[(&str, &str)] = &[
    (
        "Clamp the part as in the picture: it stands with the face of step 1 on the clamping plate; Z points up, into the part.",
        "Teil aufspannen wie im Bild: Es steht mit der Fläche aus Schritt 1 auf der Aufspannplatte; Z zeigt nach oben, ins Teil.",
    ),
    ("at least 3 points on the plate, around the part", "mindestens 3 Punkte auf der Platte, um das Teil herum"),
    (
        "The part stands on the clamping plate with this face. Probe the plate beside the part from above: at least 3 points around it, as far apart as possible. The numbered points in the picture are the ones the CNC run takes later.",
        "Das Teil steht mit dieser Fläche auf der Aufspannplatte. Die Platte neben dem Teil von oben antasten: mindestens 3 Punkte um das Teil herum, so weit wie möglich auseinander. Die nummerierten Punkte im Bild nimmt später der CNC-Lauf.",
    ),
    ("Clamping plate", "Aufspannplatte"),
    ("{n} points on the clamping plate", "{n} Punkte auf der Aufspannplatte"),
    ("Clamping plate ({face})", "Aufspannplatte ({face})"),
    ("clamping plate: point {i} is not valid", "Aufspannplatte: Punkt {i} ist ungültig"),
    ("clamping plate: point {i} lies under the part", "Aufspannplatte: Punkt {i} liegt unter dem Teil"),
    (
        "clamping plate: give at least 3 points (or none for the automatic ones)",
        "Aufspannplatte: mindestens 3 Punkte angeben (oder keine für die automatischen)",
    ),
    ("NC Program", "NC-Programm"),
    (" (for single steps also: {probes})", " (für einzelne Schritte außerdem: {probes})"),
    (" The same face also fixes the {origin}.", " Dieselbe Fläche legt auch den {origin} fest."),
    (" and ", " und "),
    (" · Position: set the alignment first", " · Position: erst die Ausrichtung festlegen"),
    ("2nd measuring length", "2. Messlänge"),
    ("2nd shaft Ø", "2. Schaft-Ø"),
    ("Alignment", "Ausrichtung"),
    ("Alignment – step by step", "Ausrichtung – Schritt für Schritt"),
    ("Angle", "Winkel"),
    ("Angle {a} – {b}", "Winkel {a} – {b}"),
    ("Approval", "Freigabe"),
    ("At the machine", "Ablauf an der Maschine"),
    ("Axis {name}", "Achse {name}"),
    ("Body {body} does not exist (any more)", "Körper {body} gibt es nicht (mehr)"),
    ("Body {body} has no faces", "Körper {body} hat keine Flächen"),
    ("Body “{body}” is missing in the model", "Körper „{body}“ fehlt im Modell"),
    ("CAD model {file}: {e}", "CAD-Modell {file}: {e}"),
    ("CNC strategy", "CNC-Strategie"),
    ("Centre {name}", "Mitte {name}"),
    ("Characteristic", "Merkmal"),
    ("Characteristics (continued)", "Prüfmerkmale (Fortsetzung)"),
    ("Characteristics in this program", "Prüfmerkmale in diesem Programm"),
    ("Check the report after the run.", "Nach dem Lauf das Protokoll prüfen."),
    ("Circle", "Kreis"),
    ("Circle Ø{d} X{x} Y{y}", "Kreis Ø{d} X{x} Y{y}"),
    ("Circular paths", "Kreisbahnen"),
    (
        "Clamp the part as in the picture: the primary plane (step 1) faces up (+Z of the machine).",
        "Teil aufspannen wie im Bild: die Primärebene (Schritt 1) zeigt nach oben (+Z der Maschine).",
    ),
    ("Created", "Erstellt"),
    ("Cylinder measurement", "Zylindermessung"),
    ("Cylinder Ø{d} X{x} Y{y}", "Zylinder Ø{d} X{x} Y{y}"),
    ("Date", "Datum"),
    ("Default: 4 points, 2 mm below the opening", "Standard: 4 Punkte, 2 mm unter der Öffnung"),
    ("Default: scan, 2 paths", "Standard: Scan, 2 Bahnen"),
    ("Design", "Design"),
    ("Deviation 0 ±{tol} (normal) · X{x} Y{y} Z{z}", "Abweichung 0 ±{tol} (normal) · X{x} Y{y} Z{z}"),
    ("Direction", "Richtung"),
    ("Direction: choose a side face (±X/±Y), not the top or bottom", "Richtung: eine Seitenfläche (±X/±Y) wählen, nicht die Ober- oder Unterseite"),
    ("Direction: choose a side face or two bores", "Richtung: eine Seitenfläche oder zwei Bohrungen wählen"),
    ("Direction: choose two bores (or a side face)", "Richtung: zwei Bohrungen wählen (oder eine Seitenfläche)"),
    ("Direction: the two bores lie on top of each other", "Richtung: die beiden Bohrungen liegen übereinander"),
    ("Distance", "Abstand"),
    ("Distance to the plane {h} mm does not fit the bore depth {d} mm", "Abstand zur Ebene {h} mm passt nicht in die Bohrungstiefe {d} mm"),
    ("Distance to the plane: between 0 and the bore depth {d} mm", "Abstand zur Ebene: zwischen 0 und der Bohrungstiefe {d} mm"),
    ("Distance {a} – {b} {axis}", "Abstand {a} – {b} {axis}"),
    ("Element", "Element"),
    ("How to", "So geht's"),
    (
        "Import the CAD model “{plan}.step” (same folder) into the plan. It already lies in the base system: do not move or realign it.",
        "CAD-Modell „{plan}.step“ (gleicher Ordner) in den Plan importieren. Es liegt schon im Basissystem: nicht verschieben, nicht neu ausrichten.",
    ),
    ("Inspection plan", "Prüfplan"),
    ("Inspection plan written ({post}): {folder}", "Prüfplan geschrieben ({post}): {folder}"),
    ("Kind", "Art"),
    ("Load the probe: {probe}{others}.", "Taster einwechseln: {probe}{others}."),
    (
        "Measurement results {plan}: {inside} characteristics in tolerance, {out} out of tolerance",
        "Messergebnis {plan}: {inside} Merkmale i. O., {out} außer Toleranz",
    ),
    ("Measurement results: {e}", "Messergebnis: {e}"),
    ("Middle {a} / {b}", "Mitte {a} / {b}"),
    (
        "Move into the orange bore ({name}) and probe at least 4 points evenly around it just below the top edge. Its centre becomes the {origin}.",
        "In die orange Bohrung ({name}) fahren und knapp unter der Oberkante mindestens 4 Punkte gleichmäßig am Umfang antasten. Ihr Mittelpunkt wird der {origin}.",
    ),
    (
        "Move into the orange bore ({name}) and probe at least 4 points evenly around it, about 2 mm below the top edge. The CNC run then measures the paths drawn in; where the axis meets the primary plane lies the {origin}.",
        "In die orange Bohrung ({name}) fahren und mindestens 4 Punkte gleichmäßig am Umfang antasten, etwa 2 mm unter der Oberkante. Die eingezeichneten Bahnen misst danach der CNC-Lauf; wo die Achse die Primärebene schneidet, liegt der {origin}.",
    ),
    ("NC program", "NC-Programm"),
    ("No.", "Nr."),
    ("Nominal / tolerance", "Sollwert / Toleranz"),
    (
        "Open the inspection plan “{plan}” in the measuring software (output: {folder}).",
        "Prüfplan „{plan}“ in der Messsoftware öffnen (Ausgabe: {folder}).",
    ),
    ("Operator", "Messtechniker"),
    ("Origin", "Nullpunkt"),
    ("Origin (middle)", "Nullpunkt (Mitte)"),
    ("Origin X", "Nullpunkt X"),
    ("Origin Y", "Nullpunkt Y"),
    ("Origin {letter}: choose a bore or a side face", "Nullpunkt {letter}: eine Bohrung oder Seitenfläche wählen"),
    ("Origin {letter}: choose a side face", "Nullpunkt {letter}: eine Seitenfläche wählen"),
    ("Origin {letter}: choose two opposite side faces", "Nullpunkt {letter}: zwei gegenüberliegende Seitenflächen wählen"),
    (
        "Origin {letter}: the side face must stand across the {letter} axis",
        "Nullpunkt {letter}: die Seitenfläche muss quer zur {letter}-Achse stehen",
    ),
    (
        "Overview: alignment faces (numbered like the steps) and the plan's coordinate system",
        "Übersicht: Ausrichtungsflächen (nummeriert wie die Schritte) und Koordinatensystem des Plans",
    ),
    ("Page {n} of {total}", "Seite {n} von {total}"),
    ("Paths: 1 to 20", "Bahnen: 1 bis 20"),
    ("Picture: {e}", "Bild: {e}"),
    ("Plane", "Ebene"),
    ("Plane inclined {n}", "Ebene schräg {n}"),
    ("Plane measurement", "Ebenenmessung"),
    ("Plane {normal} {axis}{at}", "Ebene {normal} {axis}{at}"),
    ("Point", "Punkt"),
    ("Point X{x} Y{y} Z{z}", "Punkt X{x} Y{y} Z{z}"),
    ("Point group {n} points", "Punktgruppe {n} Punkte"),
    ("Point {i}: {e}", "Punkt {i}: {e}"),
    ("Point {n}", "Punkt {n}"),
    ("Points in the picture", "Punkte im Bild"),
    ("Points per path: 3 to 100", "Punkte je Bahn: 3 bis 100"),
    ("Points: 3 to 100", "Punkte: 3 bis 100"),
    ("Postprocessor", "Postprozessor"),
    ("Postprocessor {label} did not answer within {s} s", "Postprozessor {label} hat nach {s} s nicht geantwortet"),
    ("Postprocessor {label} does not start: {e}", "Postprozessor {label} startet nicht: {e}"),
    ("Postprocessor {label}: no answer (JSON) on stdout", "Postprozessor {label}: keine Antwort (JSON) auf stdout"),
    ("Postprocessor {label}: {e}", "Postprozessor {label}: {e}"),
    ("Primary plane", "Primärebene"),
    ("Primary plane: choose a plane, not a bore", "Primärebene: eine Ebene wählen, keine Bohrung"),
    ("Primary plane: the face must look up (+Z)", "Primärebene: die Fläche muss nach oben (+Z) zeigen"),
    ("Probe", "Taster"),
    ("Probe (setup)", "Taster (Setup)"),
    (
        "Probe the base alignment by hand, in the order of the following pages, points as in the picture.",
        "Basisausrichtung von Hand antasten – in der Reihenfolge der folgenden Seiten, Punkte wie im Bild.",
    ),
    (
        "Probe the orange face ({name}) from above: at least 3 points, as far apart as possible and not on one line. The numbered points in the picture are the ones the CNC run takes later: probe close to them.",
        "Die orange Fläche ({name}) von oben antasten: mindestens 3 Punkte, so weit wie möglich auseinander und nicht auf einer Linie. Die nummerierten Punkte im Bild sind die, die der CNC-Lauf später nimmt – nahe daran antasten.",
    ),
    (
        "Probe the orange side face ({name}) square to the face, as the arrows show. It fixes the {origin}; one point is enough, two are safer.",
        "Die orange Seitenfläche ({name}) senkrecht zur Fläche antasten, wie die Pfeile zeigen. Sie legt den {origin} fest; ein Punkt genügt, zwei geben mehr Sicherheit.",
    ),
    (
        "Probe the orange side face ({name}) square to the face, as the arrows show: at least 2 points at the same height, as far apart as possible. The line through the points fixes the part's rotation about Z.{also}",
        "Die orange Seitenfläche ({name}) senkrecht zur Fläche antasten, wie die Pfeile zeigen: mindestens 2 Punkte in gleicher Höhe, so weit wie möglich auseinander. Die Linie durch die Punkte legt die Drehung des Teils um Z fest.{also}",
    ),
    ("Probe {probe} is neither a file nor in the probe table", "Taster {probe} gibt es weder als Datei noch in der Tastertabelle"),
    ("Probe: the name (probe file) is missing or a text is too long", "Taster: Name (Tasterdatei) fehlt oder ein Text ist zu lang"),
    ("Probing (CNC)", "Antasten (CNC)"),
    ("Probing by hand", "Antasten von Hand"),
    ("Remarks", "Bemerkungen"),
    ("Run order", "Ablauf"),
    ("SETUP SHEET · COORDINATE MEASURING", "EINRICHTBLATT · KOORDINATENMESSUNG"),
    ("Safety plane", "Sicherheitsebene"),
    ("Scan, {paths} paths", "Scan, {paths} Bahnen"),
    ("Setup", "Setup"),
    ("Setup sheet {file}: {e}", "Einrichtblatt {file}: {e}"),
    ("Setup sheet {plan}", "Einrichtblatt {plan}"),
    ("Setup sheet: {e}", "Einrichtblatt: {e}"),
    ("Setup: 4 points", "Setup: 4 Punkte"),
    ("Setup: circular paths", "Setup: Kreisbahnen"),
    (
        "Start the CNC run; between elements the probe retracts to the safety plane {safety}.",
        "CNC-Lauf starten; zwischen den Elementen fährt der Taster auf die Sicherheitsebene {safety}.",
    ),
    ("Step {n} · {role}", "Schritt {n} · {role}"),
    ("Task", "Aufgabe"),
    ("a plane can only be measured across it, not along {axis}", "eine Ebene lässt sich nur quer zu ihr messen, nicht entlang {axis}"),
    ("as in the tree", "wie im Baum"),
    ("at least 1 point (better 2)", "mindestens 1 Punkt (besser 2)"),
    ("at least 2 points, far apart", "mindestens 2 Punkte, weit auseinander"),
    ("at least 3 points, spread wide", "mindestens 3 Punkte, weit verteilt"),
    ("at least 4 points around the circumference", "mindestens 4 Punkte am Umfang"),
    ("at most {n} points", "höchstens {n} Punkte"),
    ("at most {n} probes", "höchstens {n} Taster"),
    ("automatic (the face's direction)", "automatisch (Flächenrichtung)"),
    ("ball Ø", "Kugel-Ø"),
    ("base body Ø", "Grundkörper-Ø"),
    ("choose a bore", "eine Bohrung wählen"),
    ("choose a bore, not a plane", "eine Bohrung wählen, keine Ebene"),
    ("choose a plane", "eine Ebene wählen"),
    ("choose a plane, not a bore", "eine Ebene wählen, keine Bohrung"),
    ("choose two different elements", "zwei verschiedene Elemente wählen"),
    ("cylinder without a valid radius", "Zylinder ohne gültigen Radius"),
    ("fixes: {what}", "legt fest: {what}"),
    ("invalid point", "ungültiger Punkt"),
    ("length", "Länge"),
    ("measuring length", "Messlänge"),
    ("no data folder", "kein Datenordner"),
    (
        "no inspection plan folder {folder} (create the plan and measure first)",
        "kein Prüfplan-Ordner {folder} (erst den Prüfplan erzeugen und messen)",
    ),
    ("no output folder", "kein Ausgabeordner"),
    ("no points: pick points on the body", "keine Punkte: Punkte auf dem Körper anklicken"),
    ("no postprocessor found", "kein Postprozessor gefunden"),
    ("no postprocessor found: put the post's program into {folder}", "kein Postprozessor gefunden: das Programm des Posts nach {folder} legen"),
    ("not a plane", "keine Ebene"),
    ("only planes and bores (cylinders) are supported", "nur Ebenen und Bohrungen (Zylinder) werden unterstützt"),
    (
        "only planes parallel to the axes (normal ±X, ±Y, ±Z) are supported",
        "nur Ebenen parallel zu den Achsen (Normale ±X, ±Y, ±Z) werden unterstützt",
    ),
    ("only vertical bores (axis Z) are supported", "nur senkrechte Bohrungen (Achse Z) werden unterstützt"),
    ("origin", "Nullpunkt"),
    ("outer cylinders (pins) are not supported yet, only bores", "Außenzylinder (Zapfen) werden noch nicht unterstützt, nur Bohrungen"),
    ("page missing", "Seite fehlt"),
    ("paths", "Bahnen"),
    ("pick at least {n} points on the face ({picked} picked)", "mindestens {n} Punkte auf der Fläche anklicken ({picked} gewählt)"),
    ("point {i} is not on the chosen face", "Punkt {i} liegt nicht auf der gewählten Fläche"),
    ("reference point", "Bezugspunkt"),
    ("rotation about Z (direction of the X axis)", "Drehung um Z (Richtung der X-Achse)"),
    ("shaft Ø", "Schaft-Ø"),
    ("sorted by probe", "nach Tastern sortiert"),
    ("spatial axis Z and origin Z", "Raumachse Z und Nullpunkt Z"),
    ("stopped without a message", "ohne Meldung abgebrochen"),
    ("the alignment has no elements yet", "die Ausrichtung hat noch keine Elemente"),
    ("the alignment's circle is missing", "Kreis der Ausrichtung fehlt"),
    ("the direction", "die Richtung"),
    ("the face has no extent", "die Fläche hat keine Ausdehnung"),
    ("the face has no triangles", "die Fläche hat keine Dreiecke"),
    ("the face is too narrow for a circular path", "die Fläche ist zu schmal für eine Kreisbahn"),
    ("the face normal is missing", "die Flächennormale fehlt"),
    ("the origin X", "der Nullpunkt X"),
    ("the origin Y", "der Nullpunkt Y"),
    ("the picture could not be made", "Bild konnte nicht erzeugt werden"),
    ("the point X{x} Y{y} Z{z} is not on the body", "der Punkt X{x} Y{y} Z{z} liegt nicht auf dem Körper"),
    ("post default", "Standardtaster des Posts"),
    ("the primary plane", "die Primärebene"),
    ("the setup measures no body yet", "das Setup misst noch keinen Körper"),
    ("the two planes are not parallel", "die beiden Ebenen sind nicht parallel"),
    ("the two planes are parallel", "die beiden Ebenen sind parallel"),
    ("there is no face of {body} at {point}", "am Punkt {point} liegt keine Fläche von {body}"),
    ("there is no room for a circular path on the face", "auf der Fläche ist kein Platz für eine Kreisbahn"),
    ("there is no room for probing points on the face", "auf der Fläche ist kein Platz für Antastpunkte"),
    ("there is room for fewer than 3 probing points on the face", "auf der Fläche ist nur Platz für weniger als 3 Antastpunkte"),
    ("{n} circular paths", "{n} Kreisbahnen"),
    ("{n} picked points", "{n} angeklickte Punkte"),
    ("{n} points", "{n} Punkte"),
    ("{n} points (picked)", "{n} Punkte (gewählt)"),
    ("{n} points (spread)", "{n} Punkte (verteilt)"),
    ("{n} points ±{tol} ({how})", "{n} Punkte ±{tol} ({how})"),
    ("{n} points, {h} mm below the opening", "{n} Punkte, {h} mm unter der Öffnung"),
    ("{paths} paths × {n} points", "{paths} Bahnen × {n} Punkte"),
    ("{plan}_setup-sheet.pdf", "{plan}_Einrichtblatt.pdf"),
    ("{probe}: {what} must be a number above 0", "{probe}: {what} muss eine Zahl über 0 sein"),
    ("{role}: choose a bore, not a plane", "{role}: eine Bohrung auswählen, keine Ebene"),
    ("{role}: choose a plane, not a bore", "{role}: eine Ebene auswählen, keine Bohrung"),
    ("{what} is missing", "{what} fehlt"),
    ("Ø{d} ±{tol} · depth {depth} · {how}", "Ø{d} ±{tol} · Tiefe {depth} · {how}"),
    ("Ø{d} ±{tol} · {n} points · {h} mm below the plane", "Ø{d} ±{tol} · {n} Punkte · {h} mm unter der Ebene"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_follow_the_language() {
        assert_eq!(Lang::from_id("de_DE.UTF-8"), Lang::De);
        assert_eq!(Lang::from_id("de-AT"), Lang::De);
        assert_eq!(Lang::from_id("en-GB"), Lang::En);
        assert_eq!(Lang::from_id("dk"), Lang::En);
        with_language(Lang::De, || {
            assert_eq!(tr!("Plane"), "Ebene");
            assert_eq!(tr!("{n} points", n = 4), "4 Punkte");
            // a missing value stays visible, a value with braces stays literal
            assert_eq!(tf("{n} points", &[]), "{n} Punkte");
            assert_eq!(tr!("{n} points", n = "{x}"), "{x} Punkte");
        });
        with_language(Lang::En, || assert_eq!(tr!("{n} points", n = 4), "4 points"));
    }

    /// Every text the CMM code passes to `tr!` has a German line, with the same placeholders.
    #[test]
    fn every_text_has_its_german_line() {
        let sources =
            [include_str!("inspection.rs"), include_str!("inspection_sheet.rs"), include_str!("inspection_results.rs"), include_str!("cmd/cmm.rs")];
        let placeholders = |s: &str| {
            let mut v: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string())).collect();
            v.sort();
            v
        };
        let mut missing = Vec::new();
        for src in sources {
            for part in src.split("tr!(\"").skip(1) {
                let Some(end) = part.find('"') else { continue };
                let en = &part[..end];
                match german().get(en) {
                    Some(de) => assert_eq!(placeholders(en), placeholders(de), "{en:?} → {de:?}"),
                    None => missing.push(en.to_string()),
                }
            }
        }
        assert!(missing.is_empty(), "no German for: {missing:#?}");
        let mut seen = std::collections::BTreeSet::new();
        for (en, _) in GERMAN {
            assert!(seen.insert(*en), "twice: {en:?}");
        }
    }
}
