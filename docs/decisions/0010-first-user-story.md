# 0010: First user story, the CNDT test block

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

Phases 1 and 2 need one real part that CNDT must design and machine end to end. The project owner asked for a part with slots, a keyway, drilled and tapped holes, chamfers, and the other features a real shop part needs.

## Decision

The first user story is the **CNDT test block**: design it in CNDT, program it in CNDT, simulate it, post it, and cut it on a real 3-axis machining centre.

**Stock:** 6061 aluminium, 100 × 60 × 25 mm. **Finished size:** 96 × 56 × 20 mm.

| # | Feature | Detail | Why it is in |
| --- | --- | --- | --- |
| 1 | Faced top | Skim to a clean surface | Facing, stock definition |
| 2 | Outside profile | 96 × 56 mm with 5 mm corner radii | Contour with corner arcs |
| 3 | Rectangular pocket | 40 × 20 × 8 mm deep, 4 mm corner radii | Pocketing, corner radius smaller than common tools |
| 4 | Closed slot | 30 mm long, 8 mm wide, 5 mm deep | Slot milling |
| 5 | Open slot | 10 mm wide, through one side, 6 mm deep | Open-ended slot, entry from outside the part |
| 6 | Keyway | 6 mm wide × 3.5 mm deep × 25 mm long, width to P9 tolerance for a 6 mm key | A width tolerance that forces roughing plus a finishing pass |
| 7 | Clearance holes | 4 × Ø6.6 through, for M6 bolts | Spot drill and drill through |
| 8 | Tapped holes | 4 × M6 × 1.0, 12 mm thread depth | Tap drill Ø5.0, rigid tapping, thread spec carried from the design to CAM |
| 9 | Counterbored holes | 2 × for M5 cap screws: Ø9.5 × 5.4 mm counterbore, Ø5.5 through | Counterbore hole type |
| 10 | Dowel hole | 1 × Ø8 H7, reamed | Ream cycle, tight tolerance |
| 11 | Chamfers | 0.5 mm × 45° on the top outside edges and every hole | Chamfer milling |
| 12 | Second setup | Flip, face to 20 mm thickness, chamfer the bottom edges | Multiple setups, stock carried between setups |

**Later stretch features**, added once their phase arrives:

- Engraved text "CNDT" on the top face (Phase 4, engraving).
- A bore with an internal keyway cut by wire EDM (Phase 7). A milled part with a wire-cut feature is exactly what one CAD/CAM app with both processes is for.

## What this asks of each part of CNDT

| Area | Needed for the test block |
| --- | --- |
| Sketcher | Lines, arcs, circles, slots, rectangles, dimensions, constraints |
| Solid features | Extrude, cut, fillet, chamfer, linear and rectangular patterns |
| Hole feature | Simple, counterbored and tapped types; the thread size (M6 × 1.0) is stored on the hole, so CAM picks the tap drill and tap without retyping it |
| CAM operations | Face, contour, pocket, slot, spot drill, drill and peck drill, tap, ream, chamfer |
| Setups | Two setups, with the stock from the first carried to the second |
| Posts | Canned cycles: G81 drill, G83 peck drill, G84 rigid tap, G85 ream |
| Simulation | Stock removal, gouge checks, and a check that tapped holes were drilled first |

## Consequences

- Phase 2's scope grows from "contour, pocket, drill and face" to the operation list above. Slot, tap, ream and chamfer were already planned for v1.0 in the spec; this pulls them into the first end-to-end story.
- The post engine needs canned-cycle support before Phase 2 finishes.
- "Done" for Phase 2 means the test block has been cut on a real machine and measured: the keyway accepts a 6 mm key, the dowel pin fits the reamed hole, and an M6 bolt threads into every tapped hole.
