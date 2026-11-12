# 0006: Rust for the core

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

The core does heavy geometry and toolpath computation, must run on three desktop systems, and should compile to WebAssembly for the web version.

## Decision

The core crates are written in Rust.

## Options considered

- **Rust.** Fast, safe parallelism, no garbage-collection pauses, one codebase for desktop, CLI and WASM. Tauri is Rust-based. Chosen.
- **C++.** The most mature CAD ecosystem and native access to OpenCascade, but slower iteration and memory-safety risk.
- **Kotlin Multiplatform.** No geometry ecosystem and a poor fit for compute-heavy 3D work. Rejected.

## Consequences

- OpenCascade is C++, so the `kernel` crate needs a foreign-function boundary and a C++ build on every platform.
- Some algorithms may be prototyped in Python first, then ported.
