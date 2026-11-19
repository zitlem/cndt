# 0029: Reference geometry: work planes, work axes and work points

- **Status:** Accepted (by agent, 2026-11-19: reference geometry in the document, fixed in model space; no rule changes)
- **Date:** 2026-11-19

## Context

The specification's v1.0 history column lists reference geometry. Today a sketch can lie only on an origin plane or on a flat face of the part, and a revolve or circular pattern can turn only about a line of the sketch, an axis of the sketch plane, or the normal through a sketch point. Designers need a plane that is not on the part (a plane offset from the origin plane, to sketch a boss that floats above the base) and an axis that is not in the sketch.

## Decision

- **Reference geometry** is a list in the document, each item with an id (never reused, like sketch ids), a name and one of three kinds:
  - **work plane:** an origin plane (`xy`, `xz`, `yz`) moved along its normal by `offset` millimetres (negative goes the other way). The sketch frame is the origin plane's frame with its origin moved by the offset; sketch X and Y are the same as on the origin plane.
  - **work axis:** a point (`through`) and a direction in model coordinates. The direction is kept as given; it must not be zero.
  - **work point:** a position in model coordinates. Stored and shown; no feature uses it yet (it is there for the planned measuring and for sketch references).
- **Use:** `Plane::Work(id)` puts a sketch on a work plane. `RevolveAxis::Work(id)` revolves about a work axis, and a circular pattern may name a work axis instead of a centre point. A work axis used by a revolve must lie in the sketch's plane (parallel to it and on it); otherwise the revolve fails saying so. A circular pattern turns about the work axis wherever it is.
- **Fixed in model space:** reference geometry does not follow the part. Nothing about it can be lost, so it never goes stale; a sketch on a work plane is placed even when the part is empty.
- **Deleting** reference geometry that a sketch or feature uses is refused, naming what uses it.
- **Commands:** `reference.add_work_plane`, `reference.add_work_axis`, `reference.add_work_point`, `reference.delete`. `sketch.create` takes `work_plane`, `feature.revolve` and `feature.edit_revolve` take `axis_work`, and `feature.circular_pattern` and `feature.edit_circular_pattern` take `axis_work`. All undo.

## Options considered

- **Planes through three points, or at an angle to another plane.** More general, but needs a frame convention for every case. An offset from an origin plane covers the common need; the record can be extended with new kinds later without changing the stored ones.
- **Reference geometry as a timeline feature.** It builds nothing, so it would only add rebuild steps and reordering rules. A separate list keeps the timeline to things that change the part.
- **Planes and axes attached to model faces and edges by name.** Wanted later; this record covers the fixed kinds, and a lost-reference message would need the naming rules for edges (planned for projected edges).

## Consequences

- The stored format changes: file version 27. Older builds refuse such projects with the update message.
- Tests: a sketch on an offset work plane extruded to the right place and volume, a revolve about a work axis (volume against closed form), a refusal for an axis not in the sketch plane, a circular pattern about a work axis, deleting reference geometry in use is refused, undo, and the format round trip and version refusal.
