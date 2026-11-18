# 0026: Crash recovery snapshots

- **Status:** Proposed
- **Date:** 2026-11-18

## Context

A project is only written when the user saves. If the app or the machine dies
with unsaved changes, those changes are lost. The file format already writes
atomically (a temporary file renamed over the target), so a half-written file
is not the risk; missing work is.

## Decision

While a document with a file has unsaved changes, the front end asks the API
to write a **snapshot** every minute. A snapshot is an ordinary project file
written by the same atomic save to `<project>.recovery`, next to the project.
No file format change: older and newer builds read it like any project.

- `snapshot` writes it. It does nothing for a document that is saved, and for
  one that has no file yet (a new document has no place to put it).
- `save` deletes it. Throwing unsaved changes away on purpose (`open`,
  `new_document` with `discard_changes`) deletes it too.
- `recovery {path}` says whether one exists and when it was written.
- `open_recovery {path}` opens the snapshot as the project at `path`, marked
  as having unsaved changes, so the next save replaces the project file.
- `discard_recovery {path}` deletes it.

After a crash nothing deletes the snapshot, so its existence at the next open
means the last session did not end by saving or discarding. The UI asks once
after opening a project: restore or discard.

## Options considered

- **A snapshot inside the project file.** Rejected: a format change, and a
  crash during the write would put the saved project at risk.
- **A directory of snapshots in the user's data folder.** Rejected for v1.0:
  needs a per-platform path and a way to match snapshots to projects. Next to
  the file is simple and visible; a read-only folder simply gets no snapshot
  and the failure is reported.
- **Journal of commands.** Rejected: larger and fragile against format
  changes; a full snapshot is already cheap.

## Consequences

- At most one minute of work is lost to a crash, for documents that have a file.
- A new, never-saved document is not protected; the user saves it once first.
- A stray `.recovery` file may sit next to a project if the app was killed; it
  is offered again at the next open until the user decides.
