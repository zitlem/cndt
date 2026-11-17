# 0018: STL files as reference meshes

- **Status:** Accepted (proposed to the owner on 2026-11-17 as "a reference body you can see and measure, but not machine to"; the owner said "merge, continue")
- **Date:** 2026-11-17

## Context

STL is the most common mesh format: scans, 3D prints, fixtures and customer models sent as meshes. A mesh is not an exact solid. Its faces are flat triangles, it may have gaps or overlaps, and it has no units. Joining it to the B-rep part would give a solid that is only as good as the triangles, and machining to it would put the tessellation's error into the G-code without saying so.

## Decision

An STL file becomes a **reference mesh**: a timeline feature, `{"type": "reference", "name", "source", "bytes", "hash"}`, that is shown and measured but is never part of the solid.

- **Shown** see-through in the 3D view, under its own Meshes layer, which can be hidden. **Measured:** its size along X, Y and Z, its corners, and its triangle count are in the browser and the state.
- **Never machined.** CAM, simulation, verification and export only see the solid. A mesh adds no faces, so nothing can reference it by name.
- **Kept in the project**, as decision 0017 keeps imported solids: the exact bytes as `design/imports/<feature id>.stl`, checked by length and hash on opening (file version 19).
- **Read** in our own code (`geom::stl`), binary or ASCII. A binary file is recognised by its length (84 bytes plus 50 per triangle), since some start with `solid` too. Millimetres are assumed, because STL carries no units. Values that are not finite numbers, facets without three corners, files cut short, and more than 10 million triangles are refused with the reason. Triangles are kept as they are, not welded or repaired.
- **Rebuilds like any feature**: undo, delete, rollback. A file that stops reading is flagged as failed, and the rest still builds. The last four meshes read are cached, so editing elsewhere does not re-read a large file.

## Options considered

- **Convert the mesh to a B-rep solid** (one planar face per triangle, sewn). It works for small clean meshes, but a scan of a million triangles makes a solid every later operation crawls through, and an open mesh fails to sew at all.
- **A display-only preview, not saved** (as STEP was before decision 0017). Useless for measuring against the design across sessions.
- **A reference feature** (chosen). Honest about what a mesh is, and saved with the project.

## Consequences

- File version 19. Version 18 builds refuse these projects with the update message.
- The `feature.reference` command is built by the `import_mesh` API request, not from the command registry, because its binary content does not fit a JSON argument. Scripts import a mesh through `import_mesh`.
- The `reference_mesh` binary call (the same layout as `body_mesh`) gives a front end the triangles.
- Machining to a mesh (for example, roughing a casting to a scan) would need its own decision.
