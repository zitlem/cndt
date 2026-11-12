# 0011: Sketch constraint solver adapted from CADCraft

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

The sketcher needs a 2D geometric constraint solver: coincident, tangent, parallel, equal, dimensions and the rest. It must report when a sketch is fully constrained, and name the constraints that conflict. The project owner chose between writing our own and using CADCraft (github.com/storytold/cadcraft).

## Findings

CADCraft's `constraints` crate, read at commit `ca87680` (2026-11-11):

- About 4,800 lines of Rust with 38 tests. No `unsafe`. Its lint rules forbid `unwrap`, `expect` and `panic`, as ours do.
- A damped Gauss-Newton solver that moves geometry as little as possible and moves just-edited geometry least.
- Inconsistent systems fail without changing anything, and the error names the conflicting constraints.
- It detects redundant constraints and names what they duplicate.
- It reports remaining degrees of freedom per object, using QR decomposition of the constraint Jacobian.
- Licensed `MIT OR Apache-2.0`, which may be combined into an AGPL program with attribution.

Drawbacks:

- It is built on CADCraft's drawing model (`cadcraft-doc`) and geometry types, so it cannot be used as a library as it stands.
- It uses `HashMap` and `HashSet` in 17 places, which our determinism rule does not allow.
- The CADCraft project is in early, fast development.

## Decision

Adapt CADCraft's solver core into our `cndt-sketch` crate:

- Keep the numerical core: residuals, the damped Gauss-Newton solver, degree-of-freedom analysis, and conflict and redundancy reporting.
- Replace its drawing model with CNDT's sketch entities, and its geometry types with `cndt-geom`.
- Replace `HashMap` and `HashSet` with ordered collections.
- Keep CADCraft's copyright and license notice in every adapted file, below our SPDX line, and add the source and commit to `docs/provenance.md` and a `NOTICE` file.
- Do not track CADCraft's changes automatically. Fixes are pulled in deliberately and recorded.

## Options considered

- **Depend on the CADCraft crate directly.** It would pull in CADCraft's whole drawing model, and its fast pace would break our build. Rejected.
- **Write our own from scratch.** Full control, but months of work to reach what this code already does and tests. Rejected for now.
- **SolveSpace or FreeCAD's planegcs.** Mature, but C++. They would need a second foreign-function bridge on three systems. Kept as the fallback if the adapted solver falls short on hard sketches.
- **Adapt CADCraft's solver.** Chosen: pure Rust, same safety rules, and the features a Fusion-style sketcher needs.

## Consequences

- The sketcher can show under-constrained, fully constrained and over-constrained states and name conflicts from day one.
- Adapted files carry two notices: our AGPL SPDX line and CADCraft's MIT/Apache notice. The combined work is AGPL-3.0-or-later.
- We own the adapted code's maintenance.
- Before adoption, the solver gets its own tests for every sketch in the test block (decision 0010).

## Implementation notes

Built in `crates/sketch` on 2026-11-12:

- Points are the only entities with coordinates. Lines, circles and arcs refer to points, so connected outlines share their corner points. Circles add a radius parameter; an arc's radius is its centre-to-start distance, with one implicit equation keeping the end on the circle.
- A line or arc tangent to an arc it shares an end point with uses the form "perpendicular to the radius at that point". The distance form is degenerate there (its error is quadratic near the solution), which left points about 3 µm off and confused the redundancy check.
- Dimensions are plain numbers at this layer. Expressions and parameters are evaluated in the model layer and passed down.
- Tests cover the test block's outside profile (96 × 56, R5 corners), closed slot and hole positions, all fully constrained.
