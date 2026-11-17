# 0017: Imported solids in the timeline

- **Status:** Accepted (the owner said "merge, continue" on 2026-11-17, with STEP import into the project next on the v1.0 list)
- **Date:** 2026-11-17

## Context

Until now a STEP file could only be looked at: triangulated for the 3D view, never part of the design. A machinist's work mostly starts from a customer's STEP file, so it has to become a solid that features, setups and toolpaths can use, and the project has to keep working when the original file is moved, changed or deleted.

## Decision

An **import feature** in the timeline: `{"type": "import", "name", "source", "operation", "bytes", "hash"}`.

- **The file is kept in the project.** Its exact content is stored as its own entry, `design/imports/<feature id>.step`, beside `design/document.json` (decision 0009, version 17). The JSON holds the file's name, its length and its 64-bit FNV-1a hash. Opening a project checks each stored copy against them, and refuses a missing, changed or unclaimed entry as damaged. The original file is never read again.
- **It rebuilds like any feature.** On every rebuild the stored content is read by the kernel into one solid (a small cache keeps the last four, so a large file is not re-read on each edit), then joined to the part or cut from it. Later features work on it as on any solid, and it can be rolled back, undone and deleted.
- **One solid.** Several solids in a file are fused into one. A file of surfaces only (common from surface modellers) is sewn at 0.001 mm and each closed shell becomes a solid. A file with no closed solid is refused, saying how many faces it held. An invalid result is repaired once with OpenCascade's shape healing, and refused if still invalid.
- **Face names** (decision 0012): `f<feature>/import/<n>`, where `n` is the face's number in the file's solid. It is the same for the same file and OpenCascade version, so sketches, chamfers and setups on imported faces survive reopening. Re-importing a changed file is a new feature, and references to the old one's faces fail visibly, as the timeline already shows for a deleted face.
- **Size.** An import may be at most 200 MB, under the project's 256 MB entry limit.
- **STEP and IGES.** STEP came first. IGES followed on 2026-11-17 (file version 18): the feature records `"format": "iges"`, its copy is stored as `design/imports/<id>.igs`, and the format is told from the content (STEP starts `ISO-10303-21`; IGES has `S` in column 73 of its first line). OpenCascade reads IGES from files only, so the stored copy is written to a private temporary file for the read and removed after. IGES files are often surfaces only, which the sewing above turns into a solid. An STL mesh is not an exact solid and would need its own decision (a reference body that can be seen and measured, but not machined to).

## Options considered

- **Reference the original file by path.** Small projects, but they break when the file moves, and a changed file silently changes the part and its toolpaths.
- **Store the solid as OpenCascade's own BREP text.** Faster to read, but ties the project file to one kernel's format, against decision 0007's rule that the kernel is replaceable.
- **Store the STEP content as-is** (chosen). It is the customer's own file, an ISO standard, and any kernel can read it.

## Consequences

- File version 17. Version 16 builds refuse these projects with the update message.
- Project files grow by the size of each imported file (compressed).
- The kernel trait gains `read_solid` (STEP or IGES data held in memory), and the build links OpenCascade's IGES library (`TKDEIGES`, or `TKIGES` before 7.8).
- The `feature.import` command carries the file's text, so undo and redo need no file.
- The `import_step` API request (STEP or IGES), and File → Import STEP or IGES into the design. Looking at a STEP file without importing it stays, as File → Look at a STEP file.
