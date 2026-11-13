# 0012: Stable references through edits, the Fusion 360 way

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

When an early feature is edited, every later feature and every toolpath that refers to a face or edge must still find the right one. This is the "topological naming" problem, and getting it wrong is the most common way parametric CAD breaks. The project owner asked for it to work the way Fusion 360 does.

Fusion 360's internals are not public, and this project does not reverse-engineer them (decision 0002). What is public is how it behaves, and its API documentation, which describes persistent "entity tokens" for faces and edges that survive recomputes. This decision matches that behaviour with our own design.

## Fusion 360's observable behaviour, which CNDT will match

1. A face or edge keeps its identity across rebuilds, so features and toolpaths that use it update instead of breaking.
2. When the design changes so much that a reference is genuinely gone, the affected feature is flagged in the timeline with a warning, instead of the model silently picking the wrong face.
3. The user repairs a flagged feature by editing it and re-selecting the reference. Everything after it then rebuilds.
4. Every face and edge can be referred to by a persistent token from the API and from scripts.

## Decision

Name every face and edge by its history:

- **Origin.** A face is named by the feature that created it and the sketch entity it was swept from. For example, the side face of an extrude is named by that extrude and the sketch line that formed it.
- **Kernel history.** After each feature, OpenCascade reports which faces and edges it generated, modified or deleted. Names are carried forward through that record, so a face split by a later cut keeps a recognisable name.
- **Stable sketch IDs.** Sketch entities keep their IDs through edits, so a face's origin survives changing a dimension.
- **Geometric check, not geometric matching.** After a rebuild, a reference is accepted only if its name resolves to exactly one face or edge. If it resolves to none, or to more than one, the feature is flagged for repair rather than guessed.
- **Repair flow.** Flagged features show a warning in the timeline. Toolpaths that depend on them are marked stale and cannot be posted until repaired.
- **Persistent tokens.** Every name has a text form, used by the file format, the API and scripts.

## Options considered

- **Match by geometry after each rebuild** (position, normal, area). Simple, but it silently picks the wrong face when the shape changes, which for CAM means cutting the wrong place. Rejected as the main method.
- **Let references break and always ask.** Simplest to build, worst to use. Kept only as the repair path for genuinely lost references.
- **History-based naming with a repair flow.** Chosen.

## Consequences

- This is designed before any feature-tree code is written in Phase 1 (spec, "Topological naming").
- The kernel bridge must expose OpenCascade's generated, modified and deleted history for every operation.
- The file format will store names. That arrives with geometry in the file and bumps the schema version.
- Safety: a toolpath never silently follows a re-guessed face. A lost reference always stops posting until a person confirms the repair.

## Implementation notes (2026-11-12)

First built with extrude, join and cut:

- Every operation in the kernel bridge returns its face history as `(source, face)` pairs. For an extrude, the source is the profile curve that swept a side face, or the start or end cap. For a boolean, it is an input face, using OpenCascade's `Modified` and `IsDeleted` records.
- A face's name is the feature that created it plus its origin: `f<feature>/side/e<curve>`, `f<feature>/start/<profile>` or `f<feature>/end/<profile>`, where a profile is named by its outer boundary's curve ids. Names are carried through every later boolean. A face merged from several keeps all of their names.
- A profile that is no longer closed after a sketch edit is not re-guessed: the feature is flagged with the reason and skipped, and the rest of the timeline still builds.
- Tests check that every name sits on exactly one face, and that the names are unchanged after a dimension edit.
- Not yet: references *to* faces (sketch on face, fillet edges, CAM). They will resolve these names, and refuse a name that matches no face or several.

Sketches on faces (2026-11-12): the first reference *to* a face. A sketch stores the face's name. At rebuild it is looked up on the part as it is just before each feature that uses the sketch (on the finished part if none does). It must match exactly one face, and that face must be flat; otherwise the sketch and its features are flagged with the reason. The sketch's axes are the model's projected onto the face, with the normal pointing out of the part, so a top-face sketch uses the model's X and Y. Test: the test block's pocket sketched on the top face still cuts correctly after the block is made taller.

Edges (2026-11-12): an edge has no name of its own; it is referred to by the faces around it. A chamfer or fillet target is either a face (every edge around it, as picking a face does in Fusion 360) or a pair of faces (the one edge where they meet). Both must resolve to exactly one face each, and a pair to exactly one edge; otherwise the feature is flagged. The face a blend makes is named after the edge's two faces, for example `f3/blend/f0/end/4-5-6-7+f1/hole/e0/wall` for the chamfer on hole 0's rim.

Patterns (2026-11-13): each copy's faces are named after the original face and the copy's place in the grid, `f<pattern>/at<i>,<j>/<original name>`. A moved copy keeps the original's face order (checked), so the names carry over exactly.
