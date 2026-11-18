# 0023: Mirror

- **Status:** Proposed
- **Date:** 2026-11-18

## Context

The specification's CAD table lists "mirror" among the v1.0 solid features. Patterns (decisions 0010 and 0022) repeat a feature by moving it; a mirrored copy is a reflection, which no rotation or translation gives. The kernel has `translate` and `rotate` only, and a rotation matrix must have determinant 1.

## Decision

Add a `mirror` timeline feature, `{"type": "mirror", "name", "feature", "plane"}`, that adds the mirror image of an earlier extrude, hole or revolve:

- **Source:** the feature it mirrors, as for the patterns. The copy joins or cuts as the original does.
- **Plane:** one of six planes, each through a fixed point and given by its normal. Through the origin of the source's sketch: `sketch_x` (normal along the sketch's X axis, so x becomes -x), `sketch_y` (y becomes -y) and `sketch_z` (the sketch plane itself, so the copy lies on the other side of it). Through the model origin: `model_x`, `model_y`, `model_z`. Planes through a model face or a work plane wait for reference geometry.
- **Kernel:** a new `Kernel::mirror(body, point, normal)` reflecting a body in the plane through `point` with unit normal `normal`, built on OpenCascade's `gp_Trsf::SetMirror` and `BRepBuilderAPI_Transform` (which reverses face orientation for a reflection). Its faces are numbered as the original's.
- **Face names:** `Copy { at: (1, 0), original }`, as for the patterns, so a mirrored face survives editing the original.
- **Failures explain themselves:** a source that did not build, comes later, or is not an extrude, hole or revolve; a plane name that does not exist; a mirror that overlaps its original so the boolean fails. Each gives a readable cause and the rest of the timeline still builds.
- **Commands:** `feature.mirror` and `feature.edit_mirror` (change the plane) in the command registry. A ribbon button follows in a later change; the command palette already lists every command.

## Options considered

- **Mirror the whole part so far** (as most CAD does by default). Handy, but it needs a body-level reflection of the running part and a rule for the seam; feature-level mirroring matches the patterns and keeps names stable. A mirror of the whole part can follow.
- **A mirror plane picked from model faces or work planes.** Needs stable plane references and reference geometry, which do not exist yet.
- **Six fixed planes through the sketch or model origin** (chosen).

## Consequences

- A new feature type changes the stored format: the file version goes up by one (version 24), and older builds refuse such projects with the update message.
- `docs/provenance.md` gets a row for the reflection.
- Tests: a mirrored extrude doubles the volume, a mirrored cut hole, a mirror of a revolve, names stable when the original is edited, undo, and the refusals with their messages.
