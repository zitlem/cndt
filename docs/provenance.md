# Algorithm provenance

Every non-trivial algorithm in CNDT is listed here with where it came from. This is the project's record that it was implemented independently from public sources.

Add a row in the same change that adds or substantially rewrites an algorithm.

## What counts as a good source

- Published papers and textbooks
- Public standards (for example ISO 6983 for G-code, ISO 10303 for STEP)
- Open-source code under an AGPL-compatible license, with project, license and version
- Vendor user documentation, read as a user would

Never: decompiled or disassembled code, leaked source, or proprietary file-format internals.

## Record

| Area | Algorithm | Crate | Source | License of source (if code) | Added |
| --- | --- | --- | --- | --- | --- |
| Sketch | Damped weighted minimum-norm Gauss-Newton (Levenberg-Marquardt style) with a central-difference Jacobian | `sketch` (`solver.rs`) | CADCraft `crates/constraints/src/lm.rs`, commit ca87680; method: K. Levenberg (1944), D. Marquardt (1963) | MIT OR Apache-2.0 (used under MIT, see `NOTICE`) | 2026-11-12 |
| Sketch | Numerical rank and null space by Householder QR with column pivoting, rows and columns balanced | `sketch` (`solver.rs`) | CADCraft `lm.rs`, commit ca87680; method: Golub and Van Loan, *Matrix Computations*, 4th ed., §5.4 | MIT OR Apache-2.0 (used under MIT) | 2026-11-12 |
| Sketch | Constraint residual equations (coincident, tangent, distance, angle and the rest) | `sketch` (`equations.rs`) | CADCraft `crates/constraints/src/residual.rs`, commit ca87680; rewritten for CNDT's point-based model | MIT OR Apache-2.0 (used under MIT) | 2026-11-12 |
| Sketch | Tangency at a shared end point as "line perpendicular to the radius" (non-degenerate form) | `sketch` (`equations.rs`) | Standard geometry; same idea as CADCraft's joined-end form in `residual.rs` | MIT OR Apache-2.0 (used under MIT) | 2026-11-12 |
| Sketch | Independent components by union-find; degrees of freedom as free parameters minus Jacobian rank, per entity as the rank of the null space restricted to its parameters; conflicts by leave-one-out solves; redundancy by rank increase | `sketch` (`system.rs`) | CADCraft `crates/constraints/src/lib.rs` and `dof.rs`, commit ca87680 | MIT OR Apache-2.0 (used under MIT) | 2026-11-12 |
| Sketch | Profile detection: closed loops as connected groups of curves where every point joins exactly two curves, walked end to end; nesting by point-in-polygon (even-odd ray casting) on sampled loops, parent = smallest enclosing loop | `sketch` (`profiles.rs`) | Standard computational geometry, e.g. de Berg et al., *Computational Geometry*, 3rd ed., ch. 2; ray casting per Shimrat (1962) | — | 2026-11-12 |
| Model | Face naming by history: names from the creating feature and sketch curve, carried through OpenCascade's Generated/Modified/IsDeleted records | `model` (`features.rs`), `occt-sys` | Own design per decision 0012; OpenCascade `BRepPrimAPI_MakePrism` and `BRepAlgoAPI` history API documentation | — | 2026-11-12 |
