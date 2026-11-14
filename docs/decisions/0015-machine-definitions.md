# 0015: Machine definitions

- **Status:** Accepted (the owner named the machine for the first cut on 2026-11-13: a Mazak HCN-5000 with a Mazatrol SmoothG control, work offsets G54 or Mazatrol WPC)
- **Date:** 2026-11-13

## Context

A post decides how a program is written. It cannot know what the machine can do: how fast the spindle turns, how far each axis travels, how many tools the magazine holds, whether it can tap rigidly. The test block's defaults (decision 0013) assume a generic machine, so nothing yet stops a program that asks for 18,000 rpm on a 12,000 rpm spindle. Architecture rule 8 says machines are data, separate plain-text files, and that the model must describe rotary axes from the first version even where only 3-axis is implemented.

## Decision

Machines are TOML files in `machines/`, one per machine, read by `cam-core`. A definition gives:

| Section | Fields |
| --- | --- |
| `[machine]` | `id`, `name`, `layout` (`vertical` or `horizontal`: which way the spindle points), `post` (the post it is normally used with), `verified` |
| `[travel]` | `x`, `y`, `z`: how far each linear axis can move, in mm |
| `[spindle]` | `max_rpm`, `taper`, `rigid_tapping` |
| `[magazine]` | `pockets`: the highest tool number |
| `[[rotary]]` | Optional rotary axes: `axis`, the linear axis it turns `around`, and `index` (the smallest step in degrees, 0 for a fully interpolating axis). Described now, not yet used by any operation |

Before posting, a program can be checked against a machine. The check refuses:

- a spindle speed above `max_rpm`
- a tool number above `pockets` or below 1
- tapping on a machine without `rigid_tapping`
- a program whose extent along any axis is longer than that axis's travel.

The extent check cannot tell whether the part sits where the program can reach it. That needs the work offset's position in machine coordinates, which only the machine knows. It catches a program that cannot fit anywhere.

Unknown fields fail to load, as with posts. Values that came from public listings rather than the machine in hand say so in the file, and a definition is `verified = false` until its values have been checked on the machine.

The first definition is `machines/mazak-hcn-5000.toml` with the `mazak-smoothg` post. The project file does not store a machine yet: the machine is chosen when posting, as the post is. Embedding a copy in the project (architecture, "Files") comes with its own format version when a project needs it.

## Options considered

- **Limits inside the post file.** One file per machine and controller pair. Rejected: the same post serves many machines, and rule 8 keeps machines separate.
- **A full kinematic chain now.** Needed for 4- and 5-axis. Rejected for now: no operation uses a rotary axis yet. The `[[rotary]]` list is the day-one interface; the chain grows from it.
- **Check travel against machine coordinates.** Needs the work offset in machine coordinates, which the operator sets at the machine. Deferred to simulation (Phase 3).

## Consequences

- `cam-core` gains `toml`, already a workspace dependency of `post`.
- The `post` API request takes an optional `machine`; with one, a program that breaks a limit produces no G-code.
- Phase 3 simulation reads the same files for travel and collision checks.
