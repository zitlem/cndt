# CNDT

Say it "sent it".

> Working name. Early development: it designs and programs a test part end to end, but nothing it produces has been proven on a machine yet. Do not run its G-code on a machine.

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
| 0 | Foundations: workspace, CI, geometry kernel build, command engine, desktop shell | Done |
| 1 | CAD core: sketcher, solid features, timeline | Built: sketches with a constraint solver (lines, arcs, circles, slots, fit-point splines, trim, offset, mirror, patterns, projected model edges, DXF import and export); extrude, cut, revolve, holes, chamfer, fillet, shell, mirror, linear and circular patterns, imported STEP solids; parameters; work planes, axes and points; suppress and reorder in the timeline; undo. Not yet: combining several bodies (needs a design decision), a solver constraint for spline end tangents, drawing splines in the UI |
| 2 | 2.5D milling and first posts | Built: face, contour, pocket, holes (spot, drill, tap, ream, counterbore), bore, slot and chamfer milling, feature detection, setups on the part's sides (by hand or on a B table, turned to any angle about the upright axis), machine definitions and a tool library, and Fanuc, Haas, LinuxCNC, Mazak SmoothG and GRBL posts that can extend one another. Unverified until it cuts a real part. Not yet: adaptive clearing, rest machining, 3D milling, turning |
| 3 | Simulation and verification (v1.0) | Built: stock simulation of every setup with the vise and carried stock; collisions, gouges, rapids through material, material left and machine limits; the posted G-code read back and compared with the plan (material, move kinds and feeds); post gates with a recorded override; prove-out (air cut) output; cancel and progress for long jobs. Not yet: spindle collisions, clamps and fixture plates, travel against the machine's measured coordinates, any post marked verified |

Also in the app: a command palette, stale markers on operations after a design edit, toolpath scrubbing, project thumbnails and crash recovery. Every feature is a command listed by `cndt commands`, and the decisions behind them are [0017 to 0035](docs/decisions/README.md).

## Building

You need:

- [Rust](https://rustup.rs). The exact version is pinned in `rust-toolchain.toml` and installs automatically.
- A C++ compiler (Xcode command-line tools, GCC or Clang, or Visual Studio).
- [Node.js](https://nodejs.org) 22 or later, for the desktop app's interface.
- On Linux, Tauri's system libraries: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libxdo-dev libssl-dev`.
- [OpenCascade](https://dev.opencascade.org) 7.9.3, the geometry kernel:
  - macOS: `brew install opencascade`
  - Linux and Windows: `micromamba create -n occt -c conda-forge occt=7.9.3`, then set `OCCT_DIR` to that environment's folder.
  - On Linux, also set `RUSTFLAGS="-C link-arg=-Wl,-rpath,$OCCT_DIR/lib"` so programs find the libraries when they run.

```sh
cargo xtask ci                                     # build the interface and run every check CI runs
cargo run -p cndt-desktop                          # start the desktop app
cargo run -p cndt-cli -- info path/to/part.step    # show a STEP file's topology and size
```

The desktop app embeds the built interface, so after changing anything in `ui/`, rebuild it with `npm --prefix ui run build` (or run `cargo xtask ci`).

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
