# Decision records

One short file per decision that would be expensive to reverse. A record states the context, the decision and its consequences.

## Status values

- **Accepted**: decided by the project owner, or by the agent under the delegation rules below. Build on it. A record accepted by the agent says so in its status line, with a one-line reason.
- **Proposed**: a working assumption. Confirm it before building on it.
- **Superseded**: replaced by a later record, which it links to.

## Delegation

The loop (the agent) may accept its own decisions, except these, which stay with the owner. A record that touches any of them is written as **Proposed**, its task goes under `[!] Needs owner` in TODO.md, and nothing is built on it:

- the license (decision 0001) and the no-CLA rule;
- a new dependency of any kind;
- the post script language;
- safety checks and verified status: weakening a check, golden-file test or gate, or marking an operation or post verified;
- new processes or scope: turning, wire EDM, 3D milling, anything outside the v1.0 column of the specification;
- changes to the rules in docs/architecture.md.

The owner can overturn any agent-accepted record in review; the record is then superseded.

## Index

| # | Decision | Status |
| --- | --- | --- |
| [0001](0001-license-agpl-no-cla.md) | AGPL-3.0-or-later, DCO, no CLA | Accepted |
| [0002](0002-independent-implementation.md) | Independent, feature-based implementation | Accepted |
| [0003](0003-unified-cad-cam-scope.md) | Unified CAD + CAM, multi-process scope | Accepted |
| [0004](0004-ui-stack.md) | Tauri + TypeScript + Three.js frontend | Accepted |
| [0005](0005-engine-first-architecture.md) | Engine-first layered workspace with a command engine | Accepted |
| [0006](0006-rust-core.md) | Rust for the core | Accepted |
| [0007](0007-geometry-kernel.md) | New app on OpenCascade behind a trait | Accepted |
| [0008](0008-opencascade-bridge.md) | Own C bridge to OpenCascade 7.9.3 | Accepted |
| [0009](0009-project-file-format.md) | Project file format, version 1 | Accepted |
| [0010](0010-first-user-story.md) | First user story: the CNDT test block | Accepted |
| [0011](0011-constraint-solver.md) | Sketch constraint solver adapted from CADCraft | Accepted |
| [0012](0012-stable-references.md) | Stable references through edits, the Fusion 360 way | Accepted |
| [0013](0013-phase-2-plan.md) | Phase 2 plan: programming the test block | Accepted |
| [0014](0014-cam-data-in-the-document.md) | CAM data lives in the document | Accepted |
| [0015](0015-machine-definitions.md) | Machine definitions | Accepted |
| [0016](0016-phase-3-plan.md) | Phase 3 plan: trust before the first cut | Accepted |
| [0017](0017-imported-solids.md) | Imported solids in the timeline | Accepted |
| [0018](0018-reference-meshes.md) | STL files as reference meshes | Accepted |
| [0019](0019-side-setups.md) | Setups on the part's sides | Accepted |
| [0020](0020-vise.md) | A vise per setup, checked in the simulation | Accepted |
| [0021](0021-revolve-feature.md) | Revolve as a solid feature | Accepted |
| [0022](0022-circular-pattern.md) | Circular pattern | Accepted |
| [0023](0023-mirror.md) | Mirror | Accepted |
| [0024](0024-post-inheritance.md) | Post inheritance | Accepted |
| [0025](0025-project-thumbnail.md) | Project thumbnail | Accepted |
| [0026](0026-crash-recovery.md) | Crash recovery snapshots | Accepted |
| [0027](0027-shell.md) | Shell | Accepted |
| [0028](0028-suppress-and-reorder.md) | Suppress and reorder features | Accepted |
| [0029](0029-reference-geometry.md) | Reference geometry: work planes, work axes, work points | Accepted |
| [0030](0030-bore-large-holes.md) | Holes larger than every drill are bored | Accepted |
| [0031](0031-slot-milling.md) | A slot as wide as an end mill is milled along its centreline | Accepted |
| [0032](0032-turned-setups.md) | Setups turned to any angle about the upright axis | Accepted |
| [0033](0033-projected-edges.md) | Model edges projected into a sketch, by the two faces that meet at them | Accepted |
| [0034](0034-splines.md) | Splines in sketches: fit-point curves through sketch points | Accepted |

## Open decisions

These have no record yet. Each needs one before the work that depends on it starts.

| Decision | Needed before |
| --- | --- |
| Turning in v1.0 or right after the milling MVP | Phase 2 planning |
| Post script language (Rhai or Lua) | Post engine |
| Cloud sync and collaboration | File format freeze |
| Product name and trademark check | Public launch |

## Adding a record

Copy [template.md](template.md), take the next number, and add it to the index.
