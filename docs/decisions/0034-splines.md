# 0034: Splines in sketches

- **Status:** Accepted (by agent, 2026-11-19: a new sketch entity on the existing sketch, model and kernel paths, in the specification's v1.0 sketcher column; no dependency, no rule change, no safety effect)
- **Date:** 2026-11-19

## Context

The specification's v1.0 sketcher column lists splines. A profile with a free-form edge (a cam lobe, a blended outline, a traced shape) cannot be drawn with lines and arcs alone. Architecture rule 9 still holds: sketch geometry is parametric, and the solid is rebuilt from it.

## Decision

- **Entity:** `Geometry::Spline { through, closed }`, a fit-point spline: a smooth curve passing through three or more existing sketch points, in order. `closed` makes it also return from the last point to the first with a smooth join. It stores point ids only, so a spline moves with its points, and every point constraint and dimension works on them as on any other point.
- **Command:** `sketch.add_spline` with `sketch`, `through` (a list of positions or existing point ids) and `closed` (default false). Fewer than three points is refused before anything is made.
- **Profiles:** an open spline joins lines and arcs end to end through its first and last points. A closed spline is a loop by itself. Region detection and nesting sample the curve (centripetal Catmull-Rom through the same points, 16 samples a span); this picture only decides which region lies inside which, and is never used for the solid.
- **Kernel:** `Curve::Spline` goes to OpenCascade as one cubic B-spline interpolated through the points (`GeomAPI_Interpolate`, periodic when closed). Extrude, cut and revolve use it like any other profile curve; the faces it sweeps are named by the sketch entity, as for lines and arcs.
- **Not in this record:** a solver constraint that sets a spline end tangent (the "end tangent" of the task), spline-specific control handles, and conic or NURBS control-point splines. The solver treats the points as the only unknowns. These are follow-ups; the TODO says so.
- **Refused, with the reason:** trim and offset of a spline, and the picks that need a circle or arc (point on curve, tangent, equal radius): they say a spline is not a line, arc or circle. A spline is not cut by other curves when they are trimmed. Mirror and the sketch patterns copy a spline with its points.
- **Export:** DXF export writes a spline as short line segments along it (16 a span).

## Options considered

- **Control-point (Bezier or NURBS) splines.** Closer to what CAD packages draw, but the control points are not on the curve, so constraints on them are awkward. Fit points are what a designer dimensions.
- **Sample the sketch spline into lines before the kernel.** Easy but loses the exact curve and gives faceted walls; rejected, as for projected edges.
- **A spline solver equation (end tangent) now.** Larger and independent of the entity; kept for a follow-up so this change stays reviewable.

## Consequences

- The stored format changes: file version 30. Older builds refuse such projects with the update message.
- `kernel::Curve` and `occt_sys::Curve` are no longer `Copy`.
- Tests: sampling passes through its points and follows a circle; a closed spline is one region of about the right area, an open spline closes a profile with a line, a spline outside another is nested; the command, its refusals, undo, mirror and delete; the format round trip and version refusal; a closed spline through points on a circle extrudes to the closed-form volume within 0.5 %, and a mixed line and spline profile extrudes.
