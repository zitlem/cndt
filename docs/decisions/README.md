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

## Open decisions

These have no record yet. Each needs one before the work that depends on it starts.

| Decision | Needed before |
| --- | --- |
| First user story (the part that drives Phases 1 and 2) | Phase 0 |
| Team size and funding | Any schedule |
| Turning in v1.0 or right after the milling MVP | Phase 2 planning |
| Constraint solver | Sketcher |
| Topological naming approach | Feature tree |
| Post script language (Rhai or Lua) | Post engine |
| Cloud sync and collaboration | File format freeze |
| Product name and trademark check | Public launch |

## Adding a record

Copy [template.md](template.md), take the next number, and add it to the index.
