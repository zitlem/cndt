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
| Model | ISO metric coarse thread sizes (M1.6 to M24) and tap drill = major diameter − pitch | `model` (`threads.rs`) | ISO 261 (general plan), ISO 724 (basic dimensions); tap drill rule as in *Machinery's Handbook*, "Tap drill sizes" | — | 2026-11-12 |
| Model | Hole cutting tool as a revolved half-section (counterbore, wall, 118° or flat point) per hole point | `model` (`features.rs`), `occt-sys` | Own design; OpenCascade `BRepPrimAPI_MakeRevol` documentation | — | 2026-11-12 |
| Kernel | Revolve face history: OpenCascade's `Generated` record, plus, for faces it does not report (a flat face square to the axis), the profile segment whose midpoint lies on the face (`BRepExtrema_DistShapeShape`) | `occt-sys` (`occt_shim.cpp`) | Own design; exact because each segment sweeps exactly one surface through its own points | — | 2026-11-12 |
| Model | Edge references by the two faces they join; blend faces named by those faces | `model` (`features.rs`), `occt-sys` | Own design per decision 0012; OpenCascade `BRepFilletAPI_MakeChamfer`, `BRepFilletAPI_MakeFillet` and `TopExp::MapShapesAndAncestors` documentation | — | 2026-11-12 |
| Model | Expression evaluation (numbers, parameter names, + − × ÷, unary minus, brackets) by recursive descent | `model` (`expr.rs`) | Standard technique, e.g. Aho, Lam, Sethi and Ullman, *Compilers*, 2nd ed., §2.4 | — | 2026-11-13 |
