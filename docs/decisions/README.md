# Decision records

One short file per decision that would be expensive to reverse. A record states the context, the decision and its consequences.

## Status values

- **Accepted**: decided by the project owner. Build on it.
- **Proposed**: a working assumption. Confirm it before building on it.
- **Superseded**: replaced by a later record, which it links to.

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
