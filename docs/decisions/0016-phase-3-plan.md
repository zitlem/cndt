# 0016: Phase 3 plan: trust before the first cut

- **Status:** Accepted (the owner said "continue" on 2026-11-14 when asked whether to start Phase 3 before the first cut)
- **Date:** 2026-11-14

## Context

Phase 2 is built, but its exit gate is a real cut on the owner's Mazak HCN-5000, with an unverified post and unverified machine values. The spec's Phase 3 ("Trust") is stock simulation, gouge and collision checks, a verification report and a post editor. Doing it before the first cut makes that cut safer: a program that crashes or gouges in simulation is caught on screen, not on the machine.

## Decision

Build Phase 3 in thin slices, each usable on its own:

1. **Stock simulation of one setup** ← first. A height map (Z-map) of the raw stock in the setup's program coordinates, cells of 0.25 mm by default, swept by each tool's shape along the toolpath: flat for end mills, face mills and reamers; a 118° cone for drills; a 90° cone for spot drills and chamfer mills. Taps are swept at their tap drill's diameter, because threads are not modelled. Checks:
   - **Collision:** a rapid move that removes material, measured against the stock before that move.
   - **Gouge:** a cut that goes below the finished part by more than 0.01 mm. The part's height in each cell comes from rays against the exact B-rep, not a mesh, so tessellation can never cause a false gouge.
   - **Material left** on the part, with where the most is.
   - **Lengths and cutting time** at the programmed feeds.

   The result goes to the API as a `simulate` request, and to the 3D view as a coloured stock mesh: clean, left, gouged and off the part.
2. **Verification report:** all setups, the machine check (decision 0015) and the simulation in one page, saved with the program. Built: a `verify` request sorts findings into problems and review items; `post` runs it first and refuses problems without an explicit override, which the program header records (docs/safety.md, gate 4).
3. **Tool assemblies:** flute length, shank and holder in the tool library (a file-format version), and holder and shank collisions in the simulation. Built (file version 16): each tool may carry a measured assembly; without one, typical sizes for its kind and diameter are assumed and the report says so. Before each point of a move cuts, anything standing above the flutes across the cutter's width is a shank rub, and anything above the holder's face within its diameter a holder crash. Both are problems.
4. **Stock carried between setups:** a three-direction dexel model, so the second side starts from what the first left.
5. **Post editor** in the app, over the same TOML files, with the golden-file preview.

## Options considered

- **Mesh boolean simulation** (subtract swept tool solids in OpenCascade). Exact, but far too slow for thousands of moves, and fragile.
- **Voxels.** Handle undercuts, but cost memory as the cube of the resolution, and a 3-axis setup cannot reach an undercut anyway.
- **Z-map** (chosen for the first slice). Fast, simple, deterministic and exact for what a 3-axis setup can reach. Its limits (one direction, no holder) are covered by slices 3 and 4.

## Consequences

- A new `sim` crate in layer 3, on `cam-core` and `geom` only. The API joins it to the kernel and the CAM plan.
- The kernel gains `heights`: rays along Z against the exact solid.
- The simulator runs optimised in debug builds too, so tests that simulate whole setups stay quick.
- Simulation finds real differences between the model and the program, not only mistakes. On the test block it reports:
  - the dowel drill's point below the modelled flat bottom
  - the chamfer cone rounding the mitred corners at the keyway
  - material left in the keyway's square inside corners

  These are reported, never hidden. Whether to change the model or the program is the user's call.
- Simulation results are UNVERIFIED like everything else until the first real cut confirms them.
