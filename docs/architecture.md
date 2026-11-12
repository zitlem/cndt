# Architecture

CNDT is an engine-first workspace. All logic lives in crates that know nothing about the user interface, and a crate may only depend on crates in lower layers.

These rules exist so that nothing has to be rewritten when the UI changes, when a new machining process is added, or when the geometry kernel is replaced.

## Layers

| Layer | Crates | Responsibility |
| --- | --- | --- |
| L0 | `geom`, `occt-sys` | 2D and 3D primitives, units, tolerances, 2D offsets. The C bridge to OpenCascade, which only `kernel` may use. |
| L1 | `kernel`, `sketch` | B-rep, booleans, fillets, STEP and IGES. 2D entities and the constraint solver. |
| L2 | `model`, `cam-core` | Feature tree, history, rebuild graph, naming. Setups, stock, machines, consumables, operations, toolpath format. |
| L3 | `process-mill`, `process-turn`, `process-wedm`, `sim`, `post`, `format` | Toolpath generators per process, simulation, post engine, project file reading and writing |
| L4 | `engine` | Command registry, undo and redo, session, background jobs |
| L5 | `api`, `mcp` | Transport-neutral API, automation |
| L6 | `ui` | Frontend |
| L7 | `apps` | Desktop, CLI and web shells |

Crates in the same layer may not depend on each other, with one exception: process modules, `sim` and `post` depend on `cam-core` types only, never on each other.

## Rules

### 1. Layering is enforced

A crate may only depend on lower layers. The check runs in CI and fails the build. The authoritative layer map is the `LAYERS` table in `xtask/src/main.rs`; keep this document in step with it. If a lower layer needs something from a higher one, define a trait in the lower layer and implement it higher up.

### 2. Commands are the only way to change state

Every change to a document is a registered command with a name, a parameter schema, an apply step and an undo step. The UI, the CLI, scripts and tests all go through the same registry. This is what gives undo and redo, scripting, automation and headless testing from one mechanism.

In code: `cndt_engine::Command` is the trait, `Session` owns the document and its history, and `Registry` builds commands by name from JSON arguments. The session only hands out `&Document`, so nothing outside a command can change it. A command's `apply` must either succeed completely or leave the document untouched.

### 3. The kernel is behind a trait

Only the `kernel` crate calls the geometry kernel, through the `occt-sys` bridge. Every other crate uses the trait and CNDT's own types. No kernel type appears in a public signature outside `kernel`. `occt-sys` is the only crate allowed `unsafe` code (decision 0008).

### 4. The core has no UI

Nothing below L6 depends on a windowing system, a browser, or the frontend framework. Everything the app can do must be possible from the CLI.

### 5. One API for the frontend

The frontend talks to the core through the `api` crate's interface. The same interface is served over desktop IPC and over direct WASM calls. No frontend code calls a transport directly.

In code: `cndt_api::Api::handle_json` takes one JSON request (`{"method": "execute", "command": ..., "args": ...}`) and returns one JSON response (`{"ok": true, "result": ...}` or `{"ok": false, "error": {"kind": ..., "message": ...}}`). Error kinds are stable identifiers front ends can branch on. Unknown fields are refused. `cndt api` serves the same protocol over standard input and output, one request per line.

### 6. The CAM core is process-agnostic

`cam-core` defines setups, stock, machines, consumables, technology tables, operations and the toolpath format. It contains nothing specific to milling, turning or wire EDM.

Each process module registers:

- its operation types, each with a parameter schema and a toolpath generator
- its consumable types
- the machine capabilities it needs
- its simulation and post hooks

Operation forms in the UI are generated from the parameter schemas.

The process-module interface stays internal and may change until a second process has shipped.

### 7. One toolpath format

A toolpath is a sequence of typed moves and process events with per-move metadata. Simulation, the post engine and the viewport all consume this format. A post never sees model geometry.

In code: `cndt_cam_core::Program` holds operations of `Step`s, each a `Move` (rapid, linear, arc, dwell) or an `Event` (tool change, spindle, coolant, stop, comment). `Program::validate` refuses unsafe programs before anything consumes them. `cndt_post::Post` loads a TOML definition from `posts/` and turns a valid program into G-code.

### 8. Machines are data

A machine definition describes axes, travel limits, the kinematic chain, capabilities and controller dialect. The model must be able to describe 3-axis mills, lathes, rotary axes, and the UV axes of a wire machine from the first version, even where only 3-axis is implemented.

### 9. References are names, not copies

Features and CAM operations refer to faces and edges by stable name. They do not store copied geometry. A design edit marks dependents stale and regenerates them.

### 10. Units and tolerances

- Lengths are millimetres as 64-bit floats everywhere inside the core.
- Inch is a display setting and a post-output setting.
- Modeling tolerance and toolpath chord tolerance are explicit values, never hidden constants.

### 11. Determinism

The same inputs produce byte-identical toolpaths and G-code on every platform. Toolpath and post code must not depend on hash-map iteration order, thread scheduling, wall-clock time, locale, or platform-specific floating-point behaviour.

### 12. Binary geometry transport

Meshes and toolpaths cross to the frontend as binary buffers. JSON is for commands and small results only.

### 13. Long work is a job

Rebuilds, toolpath generation and simulation run as cancellable background jobs with progress. The UI thread never blocks on them.

### 14. Failures are values

A failed feature or operation returns a readable cause and leaves the rest of the document usable. Core crates do not panic on bad geometry or bad input.

## Files

- A project is a single container file with a manifest, design data, manufacturing data, per-module extension data and a regenerable cache.
- Every file carries a schema version. Newer apps migrate older files.
- Data from a module the app does not know is preserved on save.
- The cache is never the source of truth.
- Machines, posts, consumables and technology tables are separate plain-text files. A project embeds a copy of what it uses.

## Changing these rules

Open a decision record in [decisions/](decisions/README.md) that states the problem, the options and the consequence. Rules change by decision, not by exception in a single change.
