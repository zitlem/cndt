# 0007: New app on OpenCascade behind a trait

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

Parametric solid CAD needs a B-rep kernel for solids, booleans, fillets and STEP or IGES exchange. This is the hardest dependency in the project.

## Decision

CNDT is a new application that uses OpenCascade as its geometry kernel. OpenCascade is hidden behind a trait in the `kernel` crate so it can be replaced later.

## Options considered

- **New app on OpenCascade.** Owns the user experience and architecture. OpenCascade is the only mature open-source B-rep kernel. Chosen.
- **Fork FreeCAD.** Years ahead on features, but its architecture and UI make a simple modern workflow hard to retrofit.
- **Fork or build on a 2D drafting codebase.** No solids, constraints or feature history. Useful as an architecture reference only.
- **A pure-Rust kernel.** Not mature enough for production solids today.
- **A commercial kernel.** Incompatible with an open-source project.

## Consequences

- OpenCascade is LGPL-2.1 with an exception, which is compatible with AGPL-3.0.
- Building it on three systems and for WebAssembly is a major early risk. It is tackled first in Phase 0.
- Fillets and booleans fail on some geometry. Failed features must report a readable cause.
- The WebAssembly build is large, so full CAD in the browser is a later goal.
- No type from OpenCascade may appear outside the `kernel` crate.
