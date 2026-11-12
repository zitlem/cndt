# 0008: Own C bridge to OpenCascade 7.9.3

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

Decision 0007 chose OpenCascade as the geometry kernel. Rust cannot call its C++ API directly, so something has to bridge the two. The kernel will need a broad and growing surface: STEP and IGES exchange, meshing, booleans, fillets, and shape history for stable references.

## Decision

CNDT has its own thin bridge in the `cndt-occt-sys` crate:

- A C++ file (`cpp/occt_shim.cpp`) exposes plain `extern "C"` functions. Every function catches all C++ exceptions and returns a status code and a message, so nothing unwinds into Rust.
- A safe Rust API wraps those functions. This crate holds the project's only `unsafe` code, and every `unsafe` block has a written safety argument.
- Only `cndt-kernel` may depend on it. The layer check enforces this.
- OpenCascade's own console output is switched off. Failures reach Rust as values.

The tested OpenCascade version is 7.9.3 on every system: Homebrew on macOS, conda-forge on Linux and Windows.

## Options considered

- **The `opencascade-sys` and `opencascade` crates.** A ready-made binding, but it covers a fixed, curated part of OpenCascade, so most features would need changes upstream first. Rejected.
- **The `occt-sys` crate.** Builds OpenCascade from bundled source as a static library. It pins version 7.8.1 and makes every clean build slow. Kept in mind as a way to produce one pinned build later.
- **Generated bindings (`cxx`, `autocxx`, `bindgen`).** OpenCascade's heavy use of templates, handles and exceptions makes generated bindings large and fragile. A hand-written C surface stays small and only grows as features need it. Rejected for now.
- **Own thin C bridge.** Chosen.

## Consequences

- Each new kernel feature needs a C function in the shim and a safe wrapper. That is deliberate: the boundary stays small and reviewed.
- The C++ standard library and OpenCascade must come from compatible toolchains. On macOS that is why CI uses Homebrew rather than conda-forge.
- Homebrew moves to new OpenCascade versions on its own schedule, so the macOS version can drift from 7.9.3.

## Follow-up

Release builds need one OpenCascade build, pinned to an exact version and built the same way on every system. Bundling or building it from source for releases needs its own decision before the first preview release.
