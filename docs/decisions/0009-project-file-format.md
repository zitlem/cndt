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
