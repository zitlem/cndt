# 0033: Projected edges in a sketch

- **Status:** Accepted (by agent, 2026-11-19: adds a command and a file format version on the existing sketch and kernel paths; no dependency, no rule change, no safety effect)
- **Date:** 2026-11-19

## Context

The specification's v1.0 history column lists projecting model edges into a sketch, so a designer can draw on a face and reuse the outline of the part below it: a boss that follows a rim, a pocket cut a fixed distance inside an edge. Architecture rule 9 says references are names, not copies: an edge is named by the two faces that meet at it, as chamfer and fillet picks do, and a lost edge must say so rather than be guessed.

## Decision

- **Command:** `sketch.project_edge` with `sketch`, and `first` and `second`, the names of the two faces that meet at the edge. The edge is looked up on the part as it is just before the first feature that uses the sketch (the whole part if none does), is projected straight onto the sketch plane, and becomes sketch geometry.
- **What it makes:** a straight edge becomes a line; a circular edge a circle (a full ring) or an arc. Its points are held with a fix constraint, so the projection cannot be dragged away from the model. The geometry is ordinary, not construction: it can close a profile. An edge that projects to a point (square to the plane), or to anything but a line, arc or circle (an ellipse from a tilted circle, a spline), is refused saying so; splines come with their own entity type.
- **Stored:** the sketch keeps a list of projections (`first`, `second` and the id of the line, arc or circle it made). The geometry itself is stored as sketch geometry as usual.
- **Stays honest:** on every rebuild each projection is looked up again. If the two faces no longer meet at one edge, or meet at several, or the edge now projects to a different place, the features that use the sketch **fail** with a message naming the sketch and the edge and saying to project it again. The stored geometry is never silently used when it no longer matches the part. Deleting the projected entity from the sketch drops the link.
- **Precision:** the kernel hands back edge outlines as 32-bit floats, so projected positions are good to about 1e-5 mm at 100 mm; the staleness check allows 1e-3 mm.
- **API:** the engine has no kernel, so `Api` computes the shape of the edge with the kernel and passes it to the command as `shape`. Run through the engine alone (without `shape`) the command refuses, saying so.
- **Delete** of the sketch entity is the ordinary `sketch.remove_entity`; projecting the same edge twice into one sketch is refused.

## Options considered

- **Re-project on every rebuild.** Would keep geometry following the part, but changes sketch geometry under constraints and dimensions drawn on it without the user asking. Failing with a reason is safer; the user projects again.
- **Store the edge's 3D curve.** A copy of geometry, against rule 9.
- **Project any curve as a polyline.** Easy, but a pile of short lines is poor geometry to dimension and to machine.

## Consequences

- The stored format changes: file version 29. Older builds refuse such projects with the update message.
- Tests: a straight edge and a circular rim project to the right length and radius with their points fixed, and a projected rim extrudes to the closed-form volume; refusals (faces that do not meet, a missing face or sketch, an edge square to the plane, the same edge twice); a shape sent by the caller is replaced; undo; the edge moved or lost fails the features using the sketch with the reason; fitting an arc either way round, a ring, a point and an ellipse; the format round trip and version refusal.
- Not covered here: the interface (the command is reachable from the API, CLI and scripts; picking an edge in the 3D view comes later).
