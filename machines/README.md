# Machine definitions

Each `.toml` file here describes one machine: which way its spindle points, how far its axes travel, how fast its spindle turns, how many tools it holds, and its rotary axes (decision 0015). CNDT checks a program against the machine before posting and refuses one that breaks a limit.

| Machine | Post | Values checked on the machine |
| --- | --- | --- |
| `mazak-hcn-5000.toml` | `mazak-smoothg` | No |

**Unchecked values come from public listings** and are set to the most cautious figure found. Check every value against the machine's own specification before relying on it, then set `verified = true`.

## Sections

| Section | Fields |
| --- | --- |
| `[machine]` | `id`, `name`, `layout` (`vertical` or `horizontal`), `post`, `verified` |
| `[travel]` | `x`, `y`, `z` in mm |
| `[spindle]` | `max_rpm`, `taper`, `rigid_tapping` |
| `[magazine]` | `pockets`: the highest tool number |
| `[[rotary]]` | `axis`, `around` (`X`, `Y` or `Z`), `index` (smallest step in degrees, 0 for a fully interpolating axis). Used for indexing between setups |

A misspelled field fails to load, so a typo can never silently drop a limit.

## What the check catches

- any post other than the machine's own (a Fanuc post's M29 is a robot request on a Mazak)
- a spindle speed above `max_rpm`
- a tool number of 0 or above `pockets`
- tapping on a machine without rigid tapping
- a program longer along an axis than that axis travels, each side on its own work offset measured alone
- a turn of a rotary axis the machine does not have, or not in whole index steps

It does not know where the part sits on the machine, so a program that fits can still run out of travel if the work offset is near a limit. Simulation (Phase 3) will check that.
