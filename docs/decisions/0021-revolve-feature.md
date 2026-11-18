# 0021: Revolve as a solid feature

- **Status:** Accepted (proposed to the owner on 2026-11-18 as "revolve first"; the owner said "Continue")
- **Date:** 2026-11-18

## Context

The specification's CAD table lists revolve among the v1.0 solid features. The kernel can already revolve (`Kernel::revolve`), but only a closed polygon, and only to cut hole tools (decision 0010): it takes a list of points, so an arc in a profile would be a chain of straight segments, and its history names edges by position, not by the sketch curve they came from. A part with a shaft, a groove, a ball end or a turned boss needs a revolve made from a sketch's own curves, exact where the sketch is exact, with faces named so later features and CAM operations survive an edit (architecture rule 10, decision 0012).

## Decision

Add a `revolve` timeline feature, `{"type": "revolve", "name", "sketch", "profiles", "axis", "angle", "operation"}`:

- **Profile:** regions of a sketch, chosen by name as for an extrude.
- **Axis:** a line of the same sketch, or one of the sketch plane's two axes (`"x"`, `"y"`). The profile may touch the axis but not cross it; a profile that crosses is refused with the reason.
- **Angle:** degrees, more than 0 up to 360; 360 by default. An expression of the parameters may drive it, as for a distance.
- **Operation:** join or cut, as for an extrude.
- **Exact curves.** The kernel gets a second revolve that takes the profile as lines and circular arcs, so a revolved arc is a true torus or sphere, not facets. The point-list revolve for hole tools stays as it is.
- **Face names.** From the creating feature and the sketch curve that swept the face, as extrude names its side faces: `f<feature>/side/<curve>`; the start and end faces of a partial turn are `f<feature>/start/…` and `f<feature>/end/…`, like an extrude's. The history comes from OpenCascade's Generated and Modified records, with the profile-segment fallback decision 0012 already uses for faces it does not report.
- **Failure explains itself:** an open profile, a profile that crosses the axis, a zero angle, an axis that is not in the sketch, or an invalid solid each give a readable cause, and the rest of the timeline still builds.
- **Command and UI:** `feature.revolve` and `feature.edit_revolve` in the command registry (so undo, scripts and the CLI work), and a Revolve button beside Extrude in the Solid ribbon, using the same dialog pattern.

## Options considered

- **Approximate arcs by many points** (the existing kernel call). Simplest, but a revolved arc becomes facets: the model is wrong by the chord error, and toolpaths and simulation inherit it. Rejected.
- **A general sweep along any path.** More than the specification's v1.0 column asks for, and sweep is listed after v1.0. Revolve is the special case that parts need now.
- **Exact curves in a new kernel call** (chosen).

## Consequences

- A new feature type is a change to the stored format: the file version goes up by one, and older builds refuse such projects with the update message. The number is taken when the build starts, after any version a parallel change holds, so two changes never share one.
- The occt-sys shim gets one new C function (catching all exceptions) with a safe wrapper, and `docs/provenance.md` a row for OpenCascade's revolve and the history rules above.
- Tests: a shaft with a groove and a filleted end (volumes against closed-form values, face names stable across editing the groove depth), refusals with their messages, a partial turn, a cut revolve, and a round trip through save and open.
- CAM sees an ordinary solid, so detection, contour and pocket work on it unchanged; whether a cylindrical face is a hole, a boss or neither is for feature detection, not this decision.
