# 0004: Tauri + TypeScript + Three.js frontend

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

The app must run on three desktop systems, may get a web version, and needs dense CAD-style panels plus a 3D viewport.

## Decision

The desktop shell is Tauri. The frontend is TypeScript with Three.js for the viewport.

## Options considered

- **Tauri + TypeScript + Three.js.** Fastest route to a polished UI, and the frontend can be reused for the web version. Chosen.
- **Pure Rust (egui + wgpu).** One language and full rendering control, proven by other open-source CAD projects, but dense panels take more work. Kept as the fallback.
- **Qt.** Mature on desktop, poor fit for a web version. Rejected.

## Consequences

- The frontend must talk to the core through one transport-neutral API, so it works over Tauri IPC and over WASM.
- Geometry reaches the frontend as binary buffers.
- Linux uses WebKitGTK, whose WebGL support is the weakest of the three platforms. It must be tested from Phase 0.
- Because the core has no UI dependency, switching to the fallback would replace only the frontend and shell.
