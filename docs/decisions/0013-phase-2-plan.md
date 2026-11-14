# 0013: Phase 2 plan: programming the test block

- **Status:** Accepted (the owner asked on 2026-11-13 for a plan with sensible defaults and to continue; the defaults below are to be revisited when the real machine and tools are known)
- **Date:** 2026-11-13

## Context

Phase 1 models the CNDT test block end to end (decision 0010). Phase 2 must turn it into G-code that cuts it on a real 3-axis machining centre, which "done" means for decision 0010. The owner has not yet named the machine, controller or tools.

## Defaults (to revisit)

| Choice | Default | Why |
| --- | --- | --- |
| Machine | Generic 3-axis vertical machining centre, metric. The owner has since named a Mazak HCN-5000 with SmoothG (decision 0015) | The test block is a 3-axis part |
| Controller dialect | Fanuc-style (`posts/fanuc-mill.toml`), which Haas also reads | The most common dialect; canned cycles G81 to G85 |
| Second post | GRBL (`posts/grbl.toml`): cycles expanded into moves, no rigid tapping | Hobby routers; proves posts without cycles work |
| Work offset | G54, origin at the part's front-left corner, Z0 on the finished top face | Easy to set with an edge finder and a probe |
| Material | 6061 aluminium (decision 0010) | — |
| Tools | A default library: 50 mm face mill; 10, 6 and 4 mm 3-flute carbide end mills; 10 mm 90° spot drill; drills 5.0, 5.5, 6.6 and 7.8 mm; M6 × 1.0 tap; 8 mm H7 reamer; 6 mm 90° chamfer mill | Covers every test-block feature |
| Feeds and speeds | Conservative carbide-in-aluminium values per tool, editable | Safe first cut; the owner tunes them |

## Order of work (one thin slice at a time)

1. **Holes:** spot drill, drill and peck drill, rigid tap, ream, all from the hole features (the thread and fit are already on them). Canned cycles in the neutral toolpath format, emitted as G81/G83/G84/G85 with G80 by posts that have them and expanded into moves by posts that do not. Generated on request from the document and posted through the API and the CLI. ← first
2. **Where CAM data lives:** setups, stock, tool library and the operation list stored in the document, behind commands with undo (needs its own decision record and a file-format version).
3. **Facing** the stock top.
4. **2D contour** of the outside profile, with cutter compensation computed by CNDT (no G41/G42 by default).
5. **Pocket** clearing (pocket, slots, keyway), on 2D offsets from the kernel.
6. **Counterbores** by helical or circular milling.
7. **Chamfer milling** along edges.
8. **Second setup:** flip, face to thickness, chamfer the bottom edges, and the bottom slot and keyway.
9. **CAM workspace in the UI:** setups, the operation list and toolpaths drawn in 3D.

Simulation and gouge checks are Phase 3, but Phase 2 already refuses unsafe orders it can check without simulation: tapping a hole that has not been drilled, or a rapid move below the safe height.

## Safety

Every operation and both posts stay **unverified** until a real cut (docs/safety.md). Every operation that emits G-code gets a golden-file test, reviewed line by line.

## Consequences

- The neutral toolpath format gains a drilling-cycle step. It is not milling-specific: lathes drill with cycles too (architecture rule 6).
- The first slice adds no file-format change: hole programs are generated from the model on request. Storing CAM data (step 2) comes with its own decision record.
