# 0027: Shell

- **Status:** Accepted (by agent, 2026-11-19: adds a feature type on the existing kernel trait; failures explain themselves)
- **Date:** 2026-11-18

## Context

The specification's CAD table lists "shell" among the v1.0 solid features: hollow a part so that a wall of even thickness is left, with chosen faces open. Cast and moulded parts, enclosures and covers are made this way. Chamfers and fillets (decision 0012) already work on the part as it stands and pick faces by stable name; a shell does the same.

## Decision

Add a `shell` timeline feature, `{"type": "shell", "name", "faces", "thickness"}`, that hollows the part inward:

- **Faces:** the faces left open, by name, as chamfers pick faces. At least one must be picked, and at least one face must stay closed.
- **Thickness:** the wall, in millimetres, more than zero. The wall is made inside the part, so the outside does not change.
- **Kernel:** a new `Kernel::shell(body, open_faces, thickness)` built on OpenCascade's `BRepOffsetAPI_MakeThickSolid::MakeThickSolidByJoin` with a negative offset and the intersection join, so outer corners stay sharp. History sources: input face `i` is `i`; the inner wall made from face `i` is `faces + i`; the rim that closes the wall at open face `i` is `2 * faces + i`.
- **Face names:** the outer faces and the rims round the openings keep their names; each inner wall is named `f<feature>/inner/<name of the face it lies behind>`, so it survives editing earlier features as the face it follows does.
- **Failures explain themselves:** a wall thicker than the part allows (an inner wall that vanishes), a picked face that no longer exists or has split, an invalid result, and no body yet. Each gives a readable cause and the rest of the timeline still builds.
- **Commands:** `feature.shell` (`faces`, `thickness`, `name`) and `feature.edit_shell` (`feature`, `thickness`). In the interface, faces are picked in the 3D view as for a chamfer.

## Options considered

- **A thickness per face.** More general, but the kernel call and the failure cases multiply; a uniform wall covers the common parts. Can follow.
- **Shelling outward.** An outward wall is an offset; it can be added as a sign or option later.
- **Rounded outer corners (arc join).** Not chosen: sharp corners match what the sketch drew.

## Consequences

- A new feature type changes the stored format: the file version goes up by one (version 25), and older builds refuse such projects with the update message.
- `docs/provenance.md` gets a row for the thick-solid call.
- Tests: a box shelled with one open face (volume against the closed form), two open faces, a wall too thick, a hollowed extrude after a fillet keeping names, names stable under editing the original, undo, and the refusals with their messages.
