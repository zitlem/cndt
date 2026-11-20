# CNDT: what is left to finish v1.0

"Finished" here means: everything the specification lists for **v1.0** that can be built and checked
without a real machine. The first real cut and verified posts on two machines (the Phase 2 and Phase 3
exit gates) need the Mazak and a person; they are listed at the bottom and are never to be attempted
or marked done by an automated run (CLAUDE.md, safety rules).

Status marks: `[ ]` to do · `[~]` PR open, waiting for merge · `[x]` merged · `[!]` needs the owner (a
decision, a dependency or the machine); never started by the loop.

Source of the list: the specification's v1.0 columns (CAD, milling, post-processor, UI, file format)
compared with `cndt commands`, the code, and `docs/`. Re-check a line against the code before starting it.

## How every task is done (the loop follows this)

1. Work in your own fresh worktree off `origin/main`, never in another session's checkout.
2. One task per iteration. New branch `loop/<task-id>-<slug>` from a fresh `origin/main`. Read `docs/architecture.md`
   and the decisions the task touches first.
3. A decision record `docs/decisions/00NN-*.md` first when the task adds a feature type or changes the
   file format (the next free number; check `origin/main` and open PRs). Unless it touches an owner-only
   area (see "Delegation" in `docs/decisions/README.md`), mark it **Accepted (by agent, with a one-line
   reason)** and build on it. If it is owner-only, mark it Proposed, move the task under `[!] Needs owner`
   and pick another. Tasks marked `[!]` get no record and no code.
4. File format changes take the next version after the highest on `origin/main` **and** in open PRs,
   add a section to `docs/decisions/0009-project-file-format.md`, and refuse older builds' files with
   the update message (see how version 22 was done).
5. Tests are part of the change. Anything that emits G-code needs a golden file. A failed feature
   explains itself; nothing panics.
6. A row in `docs/provenance.md` for every algorithm.
7. Commit with `git commit -s`, no attribution lines. Never commit `CLAUDE.md` or `.claude/`.
8. Push the branch, open a PR against `main`, mark the task `[~]` with the PR number. **Never merge**
   unless the owner has switched on self-merge (see `.claude/auto-loop.sh`); with it on, merge only
   your own PR, only after all three CI checks passed, with `gh pr merge --merge`. Never force-push.
9. Waiting for review is not a reason to stop. While a PR is open, move on to a task that does not depend
   on unmerged work. A task depends on unmerged work if it needs the code of an open PR, or if it changes
   the file format while another open PR (yours or another session's) already holds the next version.
   Take those later; prefer tasks with no format change (B1 to B4, D1, D3, E1, E3) meanwhile. Branch every
   task from `origin/main`, never from another open branch.
10. Do not touch what the other session (cndt-b5) is working on: see `gh pr list`; at the time of
   writing that is the vise, parallels, setup sheet, `crates/sim`, `crates/api/src/simulate.rs`,
   `verify.rs`, `setup_sheet.rs` and `model/cam.rs`. If a task needs one of them, mark it `[!]` with why.

**Done check for every task** (all must hold, and be quoted in the PR):
`cargo xtask ci` exits 0 · `npm --prefix ui run build` exits 0 when `ui/` changed · the task's own tests
named below exist and pass · the page still loads headless with no `[[` marker left (when `ui/` changed).

## A. CAD: solid features (spec v1.0: shell, mirror, patterns, boolean combine; revolve is done)

- [x] **A1 Circular pattern** (PR #62, merged) (approved). Decision 0022 is drafted on branch `v1/circular-pattern-decision`
      (commit 17d5bf4, unpushed) and was **Accepted by the owner on 2026-11-18**: change its status line to
      Accepted (and the README row), build on that same branch, push it and open one PR for record and code. Done: `feature.circular_pattern`
      and `feature.edit_circular_pattern`; tests for a bolt circle of holes (volume, count), a partial
      angle, a pattern of a revolve, names stable under editing the original, refusals; file version +1.
- [x] **A2 Mirror** (PR #65) (approved; continue on the existing branch `v1/mirror`, where the work is already written and uncommitted in the worktree: do NOT start a new branch; check what is there, run `cargo xtask ci`, commit, push, open the PR). Decision first. Mirror an earlier extrude, hole or revolve across one of
      the sketch plane's axes or a plane of the origin. Done: command, tests (volume doubles, names
      carried as `Copy`), refusals, version +1.
- [x] **A3 Shell** (PR #75; format change, so not self-merged) (approved). Decision first (kernel call for a hollowed solid with picked faces open,
      thickness, failure messages for a thickness that is too large). Done: kernel shim + wrapper with
      tests, `Feature::Shell`, faces picked by name, command, UI face picking like chamfer, tests for a
      box shelled with one open face (volume against closed form), a thickness too large, names stable.
- [!] **A4 Boolean combine**. Needs the owner: the model has one body, so combining bodies needs a
      multi-body design decision first.

## B. CAD: sketcher and history (spec v1.0)

- [x] **B1 Slot** (PR #63, merged) (approved; no format change: a command that draws two lines and two arcs with the
      right constraints). Tests: the profile is closed, one region, dimensions drive it.
- [x] **B2 Trim** (PR #66; not merged: GitHub reported no ubuntu/macos/windows checks for the branch) (approved; command). Tests: trimming a line at an intersection, an arc, refusal when
      nothing intersects.
- [x] **B3 Offset** (PR #74; not merged: no CI checks reported) (approved; command using `geom`'s 2D offsets). Tests: inside and outside, arcs,
      refusal when the offset collapses.
- [x] **B4 Sketch mirror and sketch pattern** (PR #76, merged) (approved; commands that create geometry plus
      symmetric/equal constraints). Tests per command.
- [x] **B5 Timeline suppress and reorder** (PR #77; format change, not self-merged) (approved; format +1: a `suppressed` flag). Tests: a
      suppressed feature leaves the part and its names, later features that depend on it fail with the
      reason, undo, reorder refuses an order that puts a feature before its sketch or source.
- [x] **B6 Reference geometry: work planes, axes, points** (PR #78; format change, not self-merged) (approved; decision first, format +1). Sketch
      on a work plane; revolve and circular pattern about a work axis. Tests.
- [~] **B7 Projected edges into a sketch** (PR #84) (approved; decision first, format +1). Project a model edge by
      its two-face name; a lost edge says so.
- [ ] **B8 Splines** (approved; decision first, format +1: a new entity type, solver support, profile
      detection, kernel curve). The largest sketch item; do it last in B. Tests: closed spline profile,
      extrude, solver with a spline end tangent.

## C. Milling operations (spec v1.0: 2D group)

- [x] **C1 Slot operation** (PR #80, by cndt-b5, stacked on #79) (approved; decision first, format +1). Slot milling for a slot-shaped pocket
      or a through slot; detection offers it. Golden files for all five posts.
- [x] **C2 Bore operation** (PR #79, by cndt-b5) (approved; decision first, format +1). Bore or circular-pocket by helical
      interpolation for holes larger than the tool library's drills; detection offers it; golden files.

## D. Platform items the spec lists for v1.0

- [x] **D1 Command palette** (PR #64, merged) (approved; UI only). Every registered command searchable by name, Ctrl/Cmd+K,
      keyboard reachable, strings through `strings.ts`. Tests: a pure function matching names, build.
- [x] **D2 Stale markers** (PR #70) (approved). Operations affected by a design edit show a stale marker until
      regenerated: the engine computes a hash of what an operation was last planned from; the UI marks it.
      Tests for the hash changing on a relevant edit and not on an irrelevant one.
- [x] **D3 Toolpath scrubbing** (PR #68; not merged: no CI checks reported) (approved; UI). Scrub the preview by operation and by move. Build + headless
      load check.
- [x] **D4 Project thumbnail** (PR #71; not merged: no CI checks reported) (approved; format +1 or an optional extra file that old builds ignore:
      decide in the record). Written on save, shown in Open.
- [x] **D5 Crash recovery snapshots** (PR #72; not merged: no CI checks reported) (approved; decision first). Periodic snapshot next to the project,
      offered on next open after a crash; atomic writes already exist. Tests with a simulated crash.
- [x] **D6 Post inheritance** (PR #69, merged) (approved; decision first). A post may `extends` a shipped base so upstream
      fixes still apply; the post editor shows the merged result. Golden tests for an extending post.
- [~] **D7 Cancellable background jobs** (PR pending, decision 0035: simulation and verification only; cancel
      and progress through `Api::job_control` and the desktop's `job_cancel`/`job_progress`).
      Left: rebuild and toolpath planning are not interruptible, and the UI has no cancel button or
      progress bar yet (`ui/src/api.ts` wrappers and a button).
- [!] **D8 Post script hook (Rhai or Lua)**. Needs the owner: choose the language, and a new dependency
      must be approved (CLAUDE.md: ask before adding a dependency).

## E. Hygiene

- [ ] **E1 UI strings, the rest**: the unit suffixes in `main.ts`, and the visible text in `sketch.ts` and
      `viewer.ts`, into `strings.ts`. Done: the rendered page is byte-identical before and after (headless
      DOM dump), build passes.
- [ ] **E2 Spec and repo in step**: after each merged task update the status paragraph of the
      specification (Claude Docs `Lggasf4Ee1ytaEG73FnpAf`, edit with the docs tools, never recreate) and
      `docs/decisions/README.md`. `CLAUDE.md` is local only: write the proposed status text to
      `.claude/claude-md-status.txt` for the owner, do not edit CLAUDE.md.
- [x] **E3 Test gaps** (PR #73; not merged: no CI checks reported): kernel (6 tests), cam-core, post refusal paths; each new test must fail without
      the code it covers.

## F. Never by the loop (need the machine or the owner)

- [!] First real cut on the Mazak HCN-5000 from CNDT G-code (Phase 2 exit gate).
- [!] Verified posts on two machines (Phase 3 exit gate); machine and post `verified = true` flags.
- [!] Product name and trademark check, cloud sync decision, turning in v1.0 (README "Open decisions").
- [!] Phase 4 and later (adaptive clearing, rest machining, sweep, loft, turning, wire EDM): after v1.0.
