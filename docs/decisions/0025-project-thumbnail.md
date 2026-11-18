# 0025: Project thumbnail

- **Status:** Proposed
- **Date:** 2026-11-18

## Context

The Open dialog shows file names only. A shop with many projects named after part numbers cannot tell
them apart without opening each. The specification lists a project thumbnail for v1.0.

## Decision

On every save, if the part has a solid, the engine renders a small picture of it (128 by 128 pixels, a
fixed isometric view, flat shading, a software rasteriser in `cndt-api`) and stores it as the extension
entry `extensions/preview/thumbnail.png`. A project without a solid stores none, and a stale entry is
removed on save. The API gains `project_thumbnail { path }`, which reads that entry from a project file
without opening it and replies `{png_base64}`, or `{png_base64: null}` when the file has none. The app
shows it in a preview step after the user picks a file in Open, before the project is loaded.

**No file format change.** Extension entries are preserved by every build, so older builds open and
re-save such a file, carrying the picture along unchanged. The thumbnail is never the source of truth
(architecture, "Files"): it is regenerated from the part on each save and nothing reads it back into the
document. The picture is deterministic: the same part gives the same bytes. The PNG is written
uncompressed (stored deflate blocks), so no dependency is added.

## Options considered

- **A new `preview/thumbnail.png` entry outside `extensions/`.** Reads of older builds refuse unknown
  entries, so it would need a file version bump and a held version number. Not chosen.
- **A picture made by the UI from the canvas.** Needs a window and a GPU; headless saves and the CLI would
  write none. Not chosen.
- **A recent-projects list with pictures.** Remembers paths in per-user state; a later addition, which
  can use the same API method.

## Consequences

Saving costs one more triangulation of the part (a few milliseconds for typical parts). Files grow by
about 49 KB before the container's compression. The Open dialog itself stays the system's file picker;
the picture shows in the step after it. The software view is a fixed angle and does not show setups,
stock or toolpaths.
