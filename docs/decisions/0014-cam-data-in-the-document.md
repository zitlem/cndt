# 0014: CAM data lives in the document

- **Status:** Accepted (step 2 of the Phase 2 plan, decision 0013)
- **Date:** 2026-11-13

## Context

Decision 0013's first slice generated hole programs on request with a built-in setup and tool library. A shop needs to keep its setups, stock, tools and operations with the part, edit them with undo, and get the same program every time it opens the file.

## Decision

- **Place:** CAM definitions are part of the project document (`design/document.json`, under `cam`), as plain data in the model layer next to the features they refer to. They change only through `cam.*` commands, with undo, like everything else (architecture rule 2).
- **Contents:**
  - **Stock:** the part's bounding box plus allowances on the sides, top and bottom. The test block's 100 × 60 × 25 mm blank around its 96 × 56 × 20 mm part is 2 mm on the sides, 2 mm on top and 3 mm underneath.
  - **Tool library:** numbered tools with kind, diameter and cutting data. The first setup added to an empty library brings in decision 0013's defaults, which the shop then edits.
  - **Setups:** a name, the side machined (top, or bottom for the part flipped about X), the work offset (G54 to G59), and an ordered list of operations.
  - **Operations:** for now, "holes", meaning every hole that faces the spindle in that setup, planned as decision 0013 describes. Facing, contours and pockets follow as their own operation kinds.
- **Work origin:** the part's front-left corner on the side being machined, with Z0 on that face. For the bottom setup, model Y and Z are mirrored, as on the machine.
- **References:** operations refer to features and faces by the persistent names of decision 0012, so they follow design edits.
- **Toolpaths are not stored.** They are generated from these definitions, like the solid.
- **File format:** schema version 9 (decision 0009).

## Consequences

- `cndt post` and the API's `post` request program a named setup from the document. A document without setups gets a clear message, not a guessed setup.
- Process modules (milling now, turning and wire EDM later) read these definitions; nothing process-specific moves into `cam-core`.
