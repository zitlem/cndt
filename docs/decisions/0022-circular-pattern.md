# 0022: Circular pattern

- **Status:** Accepted (by the owner, 2026-11-18)
- **Date:** 2026-11-18

## Context

The specification's CAD table lists "linear and circular pattern" among the v1.0 solid features. The grid pattern (`pattern`, decision 0010) repeats an earlier extrude or hole along its sketch's X and Y. Bolt circles, spokes, gear teeth and fan blades need copies turned about an axis instead. The kernel can already rotate a body (`Kernel::rotate`, about the origin, for side setups in decision 0019), so no new kernel call is needed.

## Decision

Add a `circular_pattern` timeline feature, `{"type": "circular_pattern", "name", "feature", "count", "angle", "center"}`, that repeats an earlier extrude, hole or revolve about the **sketch's normal through a point**:

- **Source:** the feature it repeats, as for the grid pattern. Each copy joins or cuts as the original does.
- **Axis:** the normal of the source's sketch plane through `center`, a point of that sketch (by entity id) or the sketch origin when left out. This is the bolt-circle case. An axis lying in the sketch plane, or one not tied to the source's sketch, is out of scope here.
- **Count:** copies counting the original, at least 2.
- **Angle:** the angle the copies span, in degrees, more than 0 up to 360. At 360 the copies are spaced evenly right round (`360 / count` apart); at less, the first and last copy are `angle` apart and the rest evenly between (`angle / (count - 1)` apart). Positive turns counter-clockwise seen from the sketch normal's tip.
- **Rotation about a point:** a copy is the original tool body moved so the axis passes through the origin, rotated by the kernel, and moved back. The matrix is the standard axis-angle one; no new kernel call is needed.
- **Face names:** as for the grid pattern, `Copy { at: (i, 0), original }`, with `i` counting round from 1, so a copy's faces survive editing the original.
- **Failures explain themselves:** a source that did not build, comes later, or is not an extrude, hole or revolve; a centre that is not a point of the source's sketch; a count under 2; an angle outside 0 to 360; copies that overlap each other badly enough for the boolean to fail. Each gives a readable cause and the rest of the timeline still builds.
- **Commands and UI:** `feature.circular_pattern` and `feature.edit_circular_pattern` in the command registry, and a Circular pattern entry beside Pattern in the Solid ribbon, with the same dialog pattern.

## Options considered

- **Extend the grid pattern with a "circular" mode.** One feature type, but its fields (counts and spacings against count, angle and centre) would half apply in each mode, and every message would have to say which mode it meant. Rejected.
- **An axis chosen freely** (a sketch line, a model edge or a work axis). More general, but model axes need reference geometry, which the specification also lists and which does not exist yet. The normal through a point covers bolt circles and turned patterns on a face; a free axis can follow once reference geometry exists.
- **A normal through a point** (chosen).

## Consequences

- A new feature type is a change to the stored format: the file version goes up by one (version 23, after revolve's 22), and older builds refuse such projects with the update message.
- `docs/provenance.md` gets a row for the axis-angle rotation matrix (a textbook formula) and the evenly-spaced-copies rule.
- Tests: copies of a hole round a centre (volumes, hole count, positions), a partial angle, a pattern of a revolve, names stable when the original's size is edited, and the refusals with their messages.
- Linear and circular patterns together then cover the pattern row of the specification's v1.0 column. Mirror stays a separate decision.
