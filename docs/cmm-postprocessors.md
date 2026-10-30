# CMM postprocessors: writing one for your measuring machine

SolidCraft plans coordinate measurements on the model (the **CMM** workspace: setups, alignment,
measurements, characteristics) but writes no measuring machine's file format itself. Like a
CAM system hands tool paths to an NC post, it describes each setup as a **neutral CMM program**
and runs a **postprocessor** that turns it into the plan of one machine's measuring software.
After measuring, the same post reads the software's results back so SolidCraft can show them on
the part.

```
SolidCraft ──program.json──▶ post ──▶ plan in the measuring software's format
SolidCraft ◀──results JSON── post ◀── result files the measuring software wrote
```

A new machine or measuring software needs only a new post; SolidCraft does not change. This page
is everything a post author needs. A complete, small post to start from:
[`crates/inspection/examples/example_post.rs`](../crates/inspection/examples/example_post.rs).

- **In this repository:** the neutral formats (`crates/inspection`: [`program.rs`](../crates/inspection/src/program.rs),
  [`results.rs`](../crates/inspection/src/results.rs)), how SolidCraft computes the program from the
  model (`crates/engine/src/inspection.rs`), and the example post.
- **Not in this repository:** posts for particular measuring software, their templates, probe
  files, plans and results. Keep a vendor's file formats, templates and sample data out of this
  repository; a post is a separate program in your own repository (any licence you choose, any
  language).

## Where posts live and how SolidCraft finds them

A post is an executable in the user's data folder:

| System | Folder |
|---|---|
| Windows | `%LOCALAPPDATA%\SolidCraft\inspection\post\` (`.exe`, `.cmd` or `.bat`) |
| Linux | `$XDG_DATA_HOME/solidcraft/inspection/post/`, usually `~/.local/share/solidcraft/inspection/post/` (executable bit set) |
| macOS | `~/Library/Application Support/SolidCraft/inspection/post/` |

Its file name (without extension) is its name in SolidCraft. Every setup has one or more **NC
programs**; each names its post (or the first one found), the plan name, the output folder
(default `<data>/inspection/output`), the post's settings and which characteristics it outputs.

Beside it:

- `<data>/inspection/probes/`: probe configuration files (any format your post reads; SolidCraft
  offers every file there except `.json`) and the **probe table** `probe-table.json`
  (`solidcraft-probe-table/1`, edited with CMM → Probes: ball Ø, length, measuring length, shaft
  Ø, …, order number). SolidCraft draws the probe on the model with it; a probe may be a table
  row without a file, known only by its name and ball.
- `<data>/inspection/output/`: where plans are written unless an NC program says otherwise.

## The three calls

A post is run with arguments, never interactively. Its **answer is one JSON object on the last
line of stdout** (earlier lines are ignored, so logging to stdout is allowed but not
recommended). **Success is exit code 0.** On failure it exits non-zero with a short reason on
stderr; SolidCraft shows that reason to the user. stdin is closed. Text is UTF-8.

### `post --describe` (at most 15 s)

What the post offers. SolidCraft shows `options` in the NC program dialog and passes the chosen
values back in `program.post_options`.

```json
{"name": "my-machine",
 "options": [
   {"key": "sort_by_probe", "label": "Sort the run by probe", "kind": "bool", "default": "on"},
   {"key": "machine", "label": "Machine name", "kind": "text", "default": "CMM-1"}],
 "results": "solidcraft-cmm-results/1"}
```

- `kind`: `bool` (a checkbox; the value comes back as `on` / `off`) or `text`.
- `results`: present when the post implements `--results`.
- A post that cannot describe itself still works; it just has no settings.

### `post <program.json> <output folder> [--replace]` (at most 120 s)

Writes the plan. SolidCraft writes the neutral program to `<output folder>/<plan>.cmm-program.json`
and runs the post on it.

- Write the plan into `<output folder>`, usually as `<output folder>/<plan>/` (a folder or a
  file, whatever the measuring software opens).
- Without `--replace`, refuse to overwrite an existing plan (exit non-zero with a reason).
- Answer:

  ```json
  {"folder": "/path/to/output/Part_Setup_1", "skipped": ["Angle 3"], "warnings": ["no probe file: default probe"]}
  ```

  - `folder`: the plan's folder (or file). SolidCraft then moves the program JSON there and writes
    beside it the CAD model as STEP in the plan's base system (`<plan>.step`) and the setup sheet
    for the operator (`<plan>_setup-sheet.pdf`), each unless the NC program turns it off.
  - `skipped`: characteristics or elements the post cannot write (yet). Skip, don't fail: the
    user gets the rest and sees what is missing.
  - `warnings`: anything the user should check.

### `post --results <plan folder>` (at most 120 s)

Reads the results of the last run from the plan's folder (the folder from the export answer;
measuring software usually saves its results there) and answers with
`solidcraft-cmm-results/1`:

```json
{"format": "solidcraft-cmm-results/1",
 "plan": "Part_Setup_1",
 "measured": "2026-10-27T11:34:20",
 "elements": [
   {"name": "Plane +Z", "points": [{"at": [12.5, 4.0, 0.012], "dir": [0, 0, -1]}]}],
 "characteristics": [
   {"name": "Deviation P10", "actual": -0.026, "nominal": 0.0, "upper": 0.05, "lower": -0.05,
    "unit": "mm", "state": "in", "element": "Point group P10", "along": "normal"},
   {"name": "Bore 1 Ø", "actual": 10.012, "nominal": 10.0, "upper": 0.02, "lower": -0.02,
    "unit": "mm", "state": "in"}]}
```

- `elements[].name`: the element names of the program. `points[].at`: where the probe touched
  the surface (if your software reports ball centres, add the ball radius along the probing
  direction); `dir`: the probing direction (into the material, unit vector). Both in the plan's
  **base system**.
- `characteristics[]`: `actual`, `nominal`, the allowed deviations `upper ≥ lower` (relative to
  the nominal), `unit` `mm` or `deg` (convert radians), `state` `in` / `out` / `unknown`.
- `element` and `along` (optional): for a characteristic that evaluates a single point on its
  own, the point's element name and what it measures: `normal` (deviation along the outward
  surface normal, + = material too much) or `x` / `y` / `z` (a coordinate in the base system).
  SolidCraft then shows that value and tolerance at the point. Without them it shows the point's
  distance from the nominal surface, without a tolerance colour. Use these fields rather than
  encoding meaning in names: SolidCraft never parses characteristic names.
- `measured`: when the run started (ISO 8601, local time), if known.
- Limits: at most 20000 elements, characteristics and points per element; all numbers finite.

SolidCraft colours each point green (within ¾ of its tolerance), yellow (within it), red
(outside) or blue (no tolerance), and lists the characteristics under the setup.

## The neutral program (`solidcraft-cmm-program/1`)

Reference: [`crates/inspection/src/program.rs`](../crates/inspection/src/program.rs) (Rust types with
serde; posts written in Rust can depend on the `solidcraft-inspection` crate and call
`Program::validate`).

**Units and coordinates:** millimetres and degrees; every coordinate in **model coordinates**
(Z up). `base_system` says where the alignment puts the part's coordinate system; a post whose
software keeps nominals in the base system (most do) moves every point and direction there:
`BaseSystem::point(p)`, `BaseSystem::direction(v)`.

| Field | Meaning |
|---|---|
| `format` | `solidcraft-cmm-program/1` |
| `plan`, `setup` | the plan's name (usually a folder name), the setup it comes from |
| `probe_file` | the setup's probe: a file from the probes folder, or a probe table name |
| `probe_balls` | ball Ø (mm) from the probe table, by the `probe_file` text of the setup or an element |
| `post_options` | the NC program's settings for this post (`key → text`) |
| `elements` | measured elements in measuring order, alignment elements first |
| `alignment` | how the base system is built (3-2-1) |
| `characteristics` | what is evaluated, in order |
| `safety` | the box around the part (`min`, `max`) and the clearance the probe keeps between elements |
| `base_system` | `origin` and unit axes `x`, `y`, `z` of the base system, in model coordinates (optional) |

### Elements

Each element has `name` (unique), `geometry`, `strategy`, optionally `safety_plane` (`+X` … `-Z`:
where the probe retracts before and after it; absent: the post's choice, usually the side the
face looks to) and `probe_file` (its own probe; absent: the program's).

| `geometry.type` | Parameters |
|---|---|
| `plane` | `normal` (outward, away from the material), `point` (on it), `min`, `max` (the face's box) |
| `cylinder` | `top` (axis point where it starts, e.g. a bore's opening), `axis` (unit direction into it), `diameter`, `depth`, `inner` (a bore) |
| `circle` | `center`, `normal` (the axis), `diameter`, `inner` |
| `point` | `point` (on the surface), `normal` (the surface's outward normal there) |

| `strategy.type` | Parameters |
|---|---|
| `circles_on_plane` | `circles`: `[{center, diameter}]`, circular paths on a plane |
| `cylinder_scan` | `paths` circular scans along the depth, `step` mm between points, `margin` from the ends |
| `cylinder_points` | `paths` circles of `points` single points each, `margin` from the ends |
| `points` | `count`; `at`: the points on the surface where SolidCraft placed them (without `at`, spread `count` over the element). Probe against the surface normal. |

### Alignment

| Field | Values |
|---|---|
| `primary` | an upward plane: spatial axis and Z zero |
| `direction` | `{type: side, plane}` (a side plane: a ±Y face gives X, a ±X face gives Y) or `{type: line, from, to}` (first cylinder or circle to the second = +X) |
| `origin_x`, `origin_y` | `{type: side, plane}`, `{type: middle, first, second}` (symmetry plane of two opposite sides), `{type: axis, element}` (a cylinder's axis where it meets the primary plane, a circle's centre) |

### Characteristics

Each has `name`, `type` and `tol: {upper, lower}` (relative to the nominal).

| `type` | Parameters |
|---|---|
| `diameter` | `element`, `nominal` |
| `distance` | `from`, `to`, `axis` (`x` / `y` / `z` of the base system), `nominal`; centre to centre, a plane by its position along its normal |
| `angle` | `first`, `second`, `nominal` (degrees, between the planes' normals) |
| `coordinate` | `element`, `axis`, `nominal`: a circle's centre or a point in the base system |
| `deviation` | `element`: a point's deviation along its normal, nominal 0 |

New kinds will be added as new `type`s. A post must not fail on a type it does not know: list it
in `skipped`.

### What SolidCraft makes of each CMM function

| Function | Elements | Characteristics |
|---|---|---|
| Cylinder | `cylinder` with `cylinder_scan`, `cylinder_points` or `points` | `diameter` |
| Circle | `circle` a set distance below the bore's opening, `points` | `diameter`, with position `coordinate` X, Y |
| Plane | (how the plane is probed wherever it is used) | – |
| Distance | the two planes or cylinders | `distance` |
| Angle | two planes (inclined ones with `points`) | `angle` |
| Points | one `point` element per point (several points: `<name> P1`, `P2`, …) | per point: check “normal”: `deviation` (a post may evaluate a point whose normal runs along an axis, within 1°, as that coordinate instead); check “coordinates”: `coordinate` X, Y and Z |
| Alignment | its faces, probed as their measurements say, else planes with the setup's default and bores scanned | – |

## Writing a post, step by step

1. **Start from the example.** Copy `crates/inspection/examples/example_post.rs` into your own
   crate (depend on `solidcraft-inspection` for the types, or parse the JSON yourself in any
   language). Build, drop the executable into the post folder, choose it in an NC program and
   export: the example writes a readable `plan.txt` with every element and characteristic.
2. **Learn your software's plan format from your own plans.** Make small plans by hand in your
   measuring software (one plane, one bore, one point, one of each characteristic) and compare
   them. Write only what you understand; `skipped` the rest.
3. **Map the alignment first.** Most software builds a base system from a primary plane,
   a direction and two origins, like `alignment`. Check the result on the machine before going
   further: a wrong sign in the rotation turns the whole plan.
4. **Move coordinates into the base system** if your software keeps nominals there
   (`base_system`). Directions are rotated only, points rotated and moved.
5. **Probes:** `probe_file` is a file from the probes folder (its format is your business) or a
   probe table name with its ball in `probe_balls`.
6. **Safety:** `safety` and each element's `safety_plane` give what your software needs for
   retract moves; the post decides how.
7. **Results:** implement `--results` once plans run. Report surface points (not ball centres) in
   the base system, and set `element` / `along` for single-point characteristics.
8. **Be robust.** The program is input: check `format`, call `Program::validate` (or check
   the same: unique names, finite numbers, at most 2000 elements and characteristics), cap loops,
   never crash; a failure is a message on stderr and a non-zero exit code.

## Testing a post

- Unit-test the translation with programs built in code (see the example and the types' tests).
- Export a real part, open the plan in your measuring software, and run it on the machine with a
  part whose dimensions you know.
- Feed `--results` the result files of a run and load them in SolidCraft (NC program → Load
  Measurement Results): the coloured points must sit on the part and agree with your software's
  report.
- `cmm.program {nc}` returns the neutral program without running a post; `cmm.export {nc}`,
  `cmm.results {nc, path}` do from scripts and the control channel what the dialogs do.

## Keep it clean

Measuring software vendors' file formats, templates, sample plans, probe files and results are
theirs or your company's: keep them in your post's own repository, never in SolidCraft's. Posts
are licensed by their authors; SolidCraft only runs them.
