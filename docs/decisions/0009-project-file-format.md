# 0009: Project file format, version 1

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

A project file has to survive years of format changes, never lose data, never be left half-written, and give byte-identical output for the same project so that changes can be compared and tested.

## Decision

A project is one `.cndt` file: a ZIP container.

| Entry | Content |
| --- | --- |
| `manifest.json` | `format: "cndt-project"`, `schema_version`, the writing app's version, `units: "mm"` |
| `design/document.json` | The design document as readable JSON |
| `extensions/<module>/...` | Data owned by modules |

Rules:

- **Versioned.** `schema_version` is 1. A newer file is refused with a message telling the user to update. Older files will be migrated when version 2 exists.
- **Never drop data.** Extension entries are kept byte-for-byte on save, even when no installed module understands them. Any other unknown entry makes the file refuse to open rather than be silently lost on the next save.
- **Deterministic.** Entries are sorted, timestamps fixed at 1980-01-01, permissions fixed, compression level fixed, JSON keys in a stable order. The same project always gives the same bytes.
- **Atomic save.** Data is written to `<name>.saving` next to the target, flushed to disk, then renamed over it.
- **Bounded reads.** Entries over 256 MB and files with over 10,000 entries are refused, so a malicious file cannot exhaust memory. Entry names must be safe relative paths.
- **Validated.** The document is checked against every model rule after loading.

## Options considered

- **A single JSON file.** Simple, but no place for binary data such as meshes, thumbnails or module data.
- **A folder per project.** Easy to diff, but awkward to email, attach or move as one thing.
- **SQLite.** Strong for very large data and partial updates, but opaque to diff and inspect, and harder to keep byte-identical.
- **ZIP with JSON inside.** Chosen: one file to move around, readable content inside, room for binary data.

## Consequences

- New dependency: `zip` (MIT) with deflate support, and its dependencies. All pass `cargo xtask licenses`.
- Geometry is not stored yet. When it is, the regenerable cache goes under `cache/` and never counts as the source of truth; adding it will bump the schema version.
- Cloud sync and collaboration are still open. If they are wanted, objects will need stable IDs, which also means a schema version bump.

## Version 2 (2026-11-12)

Sketches are stored in `design/document.json`, under `sketches`, keyed by sketch id, with `next_sketch` so deleted ids are never reused. Each holds a name, its origin plane (`xy`, `xz` or `yz`) and the sketch: entities, constraints and the next free ids.

- **Why a version bump:** a version 1 build would open a file with sketches, ignore them, and drop them on the next save. With the bump, it refuses the file and tells the user to update, as this record requires.
- **Migration:** a version 1 file reads as version 2 with no sketches. A version 1 file that does contain sketches is refused as damaged.
- **Unknown fields** in the document, a sketch, an entity or a constraint are refused when reading, so a newer build's additions are never silently lost.
- **Exact numbers:** JSON floats are parsed exactly (serde_json's `float_roundtrip`), so a saved project reopens bit for bit and gives the same toolpaths.

## Version 3 (2026-11-12)

The feature timeline is stored in `design/document.json` as `features`: an ordered list of `{id, feature}`, with `next_feature` so deleted ids are never reused. An extrude stores its sketch, its profiles (each named by its outer boundary's curve ids), its distance and whether it joins or cuts. The solid itself is not stored; it is rebuilt from the timeline on open. A version 1 or 2 file reads as having no features; one that contains features is refused as damaged.

## Version 4 (2026-11-12)

A sketch's `plane` may be `{"face": "<face name>"}` instead of `xy`, `xz` or `yz`: a flat face of the part by its persistent name (decision 0012). Older builds refuse such files with the update message instead of failing to read them. A version 3 or older file that contains one is refused as damaged.

## Version 5 (2026-11-12)

Hole features: `{"type": "hole", "name", "sketch", "points": [point ids], "spec": {...}}`. The spec holds the drill `diameter`, `depth` (`{"blind": mm}` or `"through"`), `tip_angle` in degrees, and optionally `counterbore` (`diameter`, `depth`), `thread` (`size` such as `M6x1.0`, `depth`) and `fit` (such as `H7`). Older builds refuse these files with the update message; a version 4 or older file containing a hole is refused as damaged.

## Version 6 (2026-11-12)

Chamfer and fillet features: `{"type": "chamfer", "name", "edges": [...], "distance"}` and `{"type": "fillet", "name", "edges": [...], "radius"}`. Each edge target is `{"face": "<face name>"}` (every edge around that face) or `{"between": ["<face name>", "<face name>"]}` (the one edge where two faces meet). Older builds refuse these files with the update message.

## Version 7 (2026-11-13)

Expressions: a sketch may hold `expressions`, a map from dimension constraint id to an expression of the document's parameters (such as `"width / 2"`), and an extrude may hold `distance_expression`. The stored values are the evaluated ones, so the geometry reads the same without re-evaluating. Older builds refuse these files with the update message.

## Version 8 (2026-11-13)

Pattern features: `{"type": "pattern", "name", "feature": <id of an extrude or hole>, "count_x", "spacing_x", "count_y", "spacing_y"}`, along the original's sketch axes. Older builds refuse these files with the update message.

## Version 9 (2026-11-13)

CAM definitions (decision 0014) under `cam`: `stock` (`side`, `top`, `bottom` allowances in mm), `tools` (by number: `kind`, `diameter`, `rpm`, `feed`, `description`), and `setups` (each `{id, setup: {name, side: "top"|"bottom", work_offset, operations: [{id, operation: {"type": "holes"}}]}}`), with `next_setup` and `next_operation`. Older builds refuse these files with the update message.

## Version 10 (2026-11-13)

Facing operations in a setup: `{"type": "face"}`, optionally with `"tool": <number>`. Older builds refuse these files with the update message.

## Version 11 (2026-11-13)

Contour operations in a setup: `{"type": "contour", "face": "<face name>"}`, optionally with `"tool"` and `"depth"` (mm below Z0). Older builds refuse these files with the update message.

## Version 12 (2026-11-13)

Pocket operations in a setup: `{"type": "pocket", "floor": "<face name>"}`, optionally with `"tool"`. Older builds refuse these files with the update message.

## Version 13 (2026-11-13)

Chamfer milling operations in a setup: `{"type": "chamfer", "face": "<face name>", "width": <mm>}`, optionally with `"tool"`. Older builds refuse these files with the update message.

## Version 14 (2026-11-13)

Pocket operations may carry a finishing allowance: `"finish": <mm>`. The roughing leaves that much on the walls and one full-depth climbing lap takes it off. Older builds refuse files that use it with the update message; files without it are unchanged.

## Version 15 (2026-11-13)

A setup may say how the part reaches it (decision 0015): `"flip_about": "y"` for a bottom side turned about model Y instead of X (as an HMC's B table turns a part standing on the pallet), and `"index": <degrees>` for the B angle when the table, not the operator, turns that side to the spindle. Both are left out when unused. Older builds refuse files that use them with the update message; files without them are unchanged.

## Version 16 (2026-11-14)

A tool may carry its measured assembly (decision 0016): `"assembly": {"flute_length", "stickout", "shank_diameter", "holder_diameter", "holder_length"}`, in millimetres from the tip. Without it, typical sizes for the tool's kind and diameter are assumed. Older builds refuse files that use it with the update message; files without it are unchanged.

## Version 17 (2026-11-17)

Import features (decision 0017): `{"type": "import", "name", "source", "operation", "bytes", "hash"}`. Each import's file content is its own entry, `design/imports/<feature id>.step`, stored exactly. Reading checks every entry against the feature's `bytes` and `hash` (64-bit FNV-1a, hex), and refuses a missing, changed or unclaimed entry as damaged.

- **Why a version bump:** a version 16 build would refuse the unknown feature type and the unknown entries; with the bump it says to update instead.
- **Migration:** none needed. A version 16 or older file that contains an import is refused as damaged.

## Version 18 (2026-11-17)

An import feature may hold `"format": "iges"` (left out for STEP), and its copy is then stored as `design/imports/<feature id>.igs`.

- **Why a version bump:** a version 17 build refuses the unknown `format` field as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 17 or older file with an IGES import is refused as damaged.

## Version 19 (2026-11-17)

Reference meshes (decision 0018): `{"type": "reference", "name", "source", "bytes", "hash"}`, the STL file's exact bytes stored as `design/imports/<feature id>.stl` and checked as imports are.

- **Why a version bump:** a version 18 build refuses the unknown feature type as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 18 or older file with a reference mesh is refused as damaged.

## Version 20 (2026-11-17)

A setup's `side` may be `front`, `back`, `left` or `right` as well as `top` and `bottom` (decision 0019).

- **Why a version bump:** a version 19 build refuses the unknown side as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 19 or older file with a side setup is refused as damaged.

## Version 21 (2026-11-18)

A setup may hold `vise`: `{"grip", "along" (left out for x), "jaw"}` (decision 0020).

- **Why a version bump:** a version 20 build refuses the unknown field as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 20 or older file with a vise is refused as damaged.

## Version 22 (2026-11-18)

A feature may be `{"type": "revolve", "name", "sketch", "profiles", "axis", "angle", "angle_expression" (left out when the angle is a plain number), "operation"}`, the axis being `{"type": "line", "entity"}`, `{"type": "sketch_x"}` or `{"type": "sketch_y"}` (decision 0021).

- **Why a version bump:** a version 21 build refuses the unknown feature type as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 21 or older file with a revolve is refused as damaged.

## Version 23 (2026-11-18)

A feature may be `{"type": "circular_pattern", "name", "feature", "count", "angle", "center" (left out for the sketch origin)}` (decision 0022).

- **Why a version bump:** a version 22 build refuses the unknown feature type as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 22 or older file with a circular pattern is refused as damaged.

## Version 24 (2026-11-18)

A feature may be `{"type": "mirror", "name", "feature", "plane"}` where `plane` is one of `sketch_x`, `sketch_y`, `sketch_z`, `model_x`, `model_y`, `model_z` (decision 0023).

- **Why a version bump:** a version 23 build refuses the unknown feature type as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 23 or older file with a mirror is refused as damaged.

## Version 25 (2026-11-18)

A feature may be `{"type": "shell", "name", "faces", "thickness"}` where `faces` is a list of face names left open and `thickness` the wall in millimetres (decision 0027).

- **Why a version bump:** a version 24 build refuses the unknown feature type as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 24 or older file with a shell is refused as damaged.

## Version 26 (2026-11-18)

A timeline entry may carry `"suppressed": true`; it is left out when `false`. A suppressed feature stays in the timeline and keeps its id, but the part is built without it (decision 0028).

- **Why a version bump:** a version 25 build refuses the unknown field as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 25 or older file with a suppressed entry is refused as damaged.

## Version 27 (2026-11-19)

The document may carry `"references"`, a map from id to reference geometry (`{"type": "work_plane", "name", "base", "offset"}`, `{"type": "work_axis", "name", "through", "direction"}` or `{"type": "work_point", "name", "position"}`), and `"next_reference"`. A sketch's plane may be `{"work": id}`, a revolve's axis `{"type": "work", "reference": id}`, and a circular pattern may carry `"axis": id` (decision 0029).

- **Why a version bump:** a version 26 build refuses the unknown fields and values as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 26 or older file with reference geometry is refused as damaged.

## Version 28 (2026-11-19)

A setup's `side` may be `turned`, with `angle` in degrees about model Y (decision 0032). Only a turned setup has an angle, and it must have one, from -360 to 360.

- **Why a version bump:** a version 27 build refuses the unknown side as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 27 or older file with a turned setup is refused as damaged.

## Version 29 (2026-11-19)

A sketch record may carry `"projections"`, a list of `{"first", "second", "entity"}`: the two face names of a projected model edge and the id of the line, arc or circle it made (decision 0033). It is left out when empty.

- **Why a version bump:** a version 28 build refuses the unknown field as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 28 or older file with projections is refused as damaged.

## Version 30 (2026-11-19)

A sketch entity may be `{"type": "spline", "through": [point ids], "closed": true}`; `closed` is left out when false (decision 0034).

- **Why a version bump:** a version 29 build refuses the unknown entity type as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 29 or older file with a spline is refused as damaged.

## Version 31 (2026-11-19)

A setup's `vise` may carry `"body"`, a number: how far below the stock's bottom the vise's floor stands, in millimetres (decision 0020). It is left out when there is none.

- **Why a version bump:** a version 30 build refuses the unknown field as damaged; with the bump it says to update instead.
- **Migration:** none needed. A version 30 or older file with a body is refused as damaged; one without is read as before.
