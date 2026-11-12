# 0005: Engine-first layered workspace with a command engine

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

The project will grow for years, add machining processes, and may change its UI or geometry kernel. The structure has to make those changes local.

## Decision

All logic lives in UI-free crates arranged in layers, where a crate may only depend on lower layers. Every change to a document goes through a command registry. The full rules are in [../architecture.md](../architecture.md).

## Consequences

- The layering check runs in CI from the first commit.
- Undo and redo, scripting, the CLI and automation all come from the command registry.
- Everything the app does is testable headless.
- Adding a process, replacing the UI or replacing the kernel touches a bounded set of crates.
- Some things take more code up front: traits at layer boundaries, and commands instead of direct edits.
