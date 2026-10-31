# Coordinate measuring (CMM workspace)

The CMM workspace plans coordinate measurements on the model and hands them to the measuring
machine through a **postprocessor**, as a CAM system does for NC machines. The plans are saved
with the design; a post writes them in your measuring software's format, and reads the results
back after measuring. SolidCraft ships no post for a particular measuring software: see
[cmm-postprocessors.md](cmm-postprocessors.md) for how to write one (an example post is included).

## Planning

Switch to the workspace with the workspace menu (Design / Render / **CMM**). The browser shows
the **CMM Plan**: setups, each with its probe system, alignment, inspection plan (the
characteristics), NC programs and, once loaded, measurement results.

1. **New Setup** (`cmm.setup`): one clamping of the part. Probe system: the probe (from the probe
   table), the safety plane the probe retracts to between elements (default +Z) and how planes are
   probed by default.
2. **Alignment** (`cmm.alignment`): 3-2-1 from faces of the model. A primary plane (spatial axis
   and Z zero), a direction (a side plane, or the line through two bores) and the origin in X and
   Y (a side plane, the middle of two opposite sides, or a bore's axis). The measuring axes follow
   the part, not the model: a part lying turned in space is aligned on its own faces, and the
   primary plane may face any way. Each characteristic is worked out in these axes, so a turned
   part gets the same names and nominal values as an upright one.
   - **Clamping plate:** when the primary face is the face the part is clamped on (`clamp: true`),
     Z points into the part, and instead of that covered face a virtual plane through it, the
     clamping plate, is probed beside the part. Its points are given in the alignment's X/Y
     (`clamp_points`, at least 3, none under the part); without them four points are placed
     `clamp_margin` mm (default 10) outside the part's corners. The setup sheet tells the
     operator to probe the plate.
3. **Characteristics:** Circle, Cylinder and Plane measurements with their probing strategy
   (circular paths, scans, single points, picked points), Distance, Angle and **Points** (one or
   many points on any surface, checked along the normal or as coordinates). Each can use its own
   probe and safety plane.
4. **Probes** (`cmm.probe.table`): the probe table with each probe's dimensions (ball Ø, length,
   measuring length, shaft Ø, …), used to draw the probe at the probing points.

Pick a characteristic in the browser to see its probing points and the probe on the model.

## Output: NC programs

A setup has one or more **NC programs** (`cmm.nc`), one per machine or post: the postprocessor,
its settings (what the post offers with `--describe`), the plan name, the output folder and
which characteristics it outputs. **Create Inspection Plan** (`cmm.export`) runs the post. Into
the plan's folder go, besides the plan:

- `<plan>.cmm-program.json`: the neutral program the post received,
- `<plan>.step`: the measured bodies moved into the plan's base system, for the measuring
  software's CAD import (optional),
- `<plan>_setup-sheet.pdf` (German: `<plan>_Einrichtblatt.pdf`): the setup sheet for the operator, with a picture per alignment step
  and the characteristics (optional; `cmm.sheet` writes it alone).

## Results

After measuring, **Results** in the ribbon or **Load Measurement Results** on an NC program
(`cmm.results`) asks the program's post to read the results in the plan's folder. Every probed
point is drawn on the part, coloured by its deviation: green within ¾ of its tolerance, yellow
within it, red outside, blue without a tolerance; points outside their tolerance are labelled
with the deviation. The browser lists each characteristic with its actual value under
**Measurement Results**; picking one highlights its point. Results are a view: they are not saved
with the design (`cmm.results {clear: true}` hides them).

## Language

Everything the workspace writes for others — element and characteristic names (they become the
names in the measuring software's plan), descriptions, messages and the setup sheet — is in
English, or in German when German is the interface language (Preferences → Language). Scripts
and the CLI follow the system language (`LANG`). Choose the language before creating a plan: the
plan keeps the names it was written with, and results are matched against the program saved in
the plan's folder.

## Folders

Everything machine-specific stays in the user's data folder, never in the design or this
repository:

| Folder (`<data>/inspection/…`) | What |
|---|---|
| `post/` | the postprocessors (executables) |
| `probes/` | probe configuration files and `probe-table.json` |
| `output/` | plans, unless an NC program names another folder |

`<data>` is `%LOCALAPPDATA%\SolidCraft` on Windows, `~/Library/Application Support/SolidCraft` on
macOS and `~/.local/share/solidcraft` on Linux.

## Scripts and the control channel

All of it is commands: `cmm.setup`, `cmm.alignment`, `cmm.circle`, `cmm.cylinder`, `cmm.plane`,
`cmm.distance`, `cmm.angle`, `cmm.point`, `cmm.nc`, `cmm.export`, `cmm.sheet`, `cmm.results`,
`cmm.program` (the neutral program without a post), `cmm.state` (all plans as JSON),
`cmm.probe.table`, `cmm.post.options`, and `cmm.setup.activate`, `cmm.setup.delete`,
`cmm.characteristic.delete`, `cmm.nc.delete`, `cmm.rename`. `solidcraft-cli commands` lists their
parameters.
