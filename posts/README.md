# Post-processors

Each `.toml` file here turns CNDT's neutral toolpaths into G-code for one controller. Posts are plain text so a shop can read, edit, version and share them.

| Post | Controller | Verified on a real machine |
| --- | --- | --- |
| `fanuc-mill.toml` | Fanuc-style milling | No |
| `grbl.toml` | GRBL routers and mills | No |

**Unverified posts have never run on a real controller.** Every program they produce says so in its header. Prove out any program carefully before running it: see [docs/safety.md](../docs/safety.md).

## Sections

| Section | What it sets |
| --- | --- |
| `[post]` | `id`, `name`, `version`, and `verified`, which may only be `true` after the post's output has cut a test part on that controller |
| `[format]` | Decimals, trailing zeros and decimal point, block numbers, comment style, `%` wrapping, program number width |
| `[program]` | An optional unnumbered `first_line`, the `start` lines (one must be exactly `{header}`), and the `end` lines |
| `[codes]` | The code for each motion and event, the `tool_change` lines, and optional `tool_length_compensation` |

## Placeholders

| Where | Available |
| --- | --- |
| `program.first_line`, `program.start` | `{program_number}`, `{program_name}`, `{work_offset}` |
| `codes.spindle_cw`, `codes.spindle_ccw` | `{rpm}` |
| `codes.dwell` | `{seconds}`, `{milliseconds}` |
| `codes.tool_change`, `codes.tool_length_compensation` | `{tool}` |

A misspelled field or placeholder makes the post fail to load, so a typo can never silently produce wrong G-code.

## Rules the engine enforces

- **Safety header.** Every program states the CNDT version, the post and its version, and whether the post is verified. Preview builds add a not-for-production notice.
- **Validated input.** The program is checked before posting: no motion before the position is known, no cutting without a tool or with the spindle off, and every arc consistent.
- **Tool length compensation.** When a post sets `tool_length_compensation`, the first move after every tool change must be a rapid. It is output as an XY move, then the compensation code with the Z move.
- **Deterministic output.** The same program and post always give the same bytes.

## Changing a post

Every shipped post has a golden-file test in `crates/post/tests/golden/`. If you change a post or the engine and the output changes on purpose, regenerate with:

```sh
CNDT_UPDATE_GOLDEN=1 cargo test -p cndt-post
```

Then review every changed line in the diff before committing.
