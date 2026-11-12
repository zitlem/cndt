# CNDT

> Working name. Not yet usable: the project is in planning and no code has been written.

CNDT is an open-source CAD/CAM application for designing parts and machining them from the same model. The goal is a simple, single-window workflow with professional machining depth.

## Goals

- **One model.** Design a part, set up the job, generate toolpaths, simulate, and post G-code in one application. Toolpaths update when the design changes.
- **Simple first, deep when asked.** Each operation starts with a few fields and working defaults. Full control is always available.
- **Trustworthy output.** Simulation and verification before any G-code leaves the app.
- **Editable posts.** Every post-processor is a plain-text file a shop can read, change and share.
- **Many processes.** Milling first, then turning, 3D milling and wire EDM on the same core.
- **Cross-platform.** Windows, macOS and Linux. A web version is planned later.

## Status

| Phase | Scope | State |
| --- | --- | --- |
| 0 | Foundations: workspace, CI, geometry kernel build, command engine | In progress |
| 1 | CAD core: sketcher, solid features, timeline | Not started |
| 2 | 2.5D milling and first posts | Not started |
| 3 | Simulation and verification (v1.0) | Not started |

## Building

You need [Rust](https://rustup.rs). The exact version is pinned in `rust-toolchain.toml` and installs automatically.

```sh
cargo xtask ci          # run every check CI runs
cargo run -p cndt-cli   # run the command-line app
```

## Safety

CNC machines can cause serious injury and damage. Until a release is marked production-ready, nothing produced by this software should be run on a machine. Even then, always verify a program before running it. See [docs/safety.md](docs/safety.md).

This software comes with no warranty. See the license.

## Documentation

- [Architecture](docs/architecture.md)
- [Decision records](docs/decisions/README.md)
- [Safety gates](docs/safety.md)
- [Algorithm provenance](docs/provenance.md)
- [Contributing](CONTRIBUTING.md)

## License

CNDT is free software, licensed under the [GNU Affero General Public License, version 3 or later](LICENSE).

It is a public project and is meant to stay that way. Contributors keep their own copyright and there is no contributor license agreement, so no single person or company can relicense or close it.

What you make with CNDT is yours. G-code, part files, and the post-processors, machine definitions and tool libraries you write for your own shop are not covered by this license.

CNDT is an independent project. It is not affiliated with, endorsed by, or derived from any commercial CAD/CAM product.
