# 0035: Cancel and progress for long work

- **Status:** Accepted (by agent, 2026-11-19: architecture rule 13 already requires cancellable jobs with progress; this builds that rule for the two slowest requests, with no new dependency, no format change and no change to a check)
- **Date:** 2026-11-19

## Context

Architecture rule 13 says long work is a cancellable job with progress and
never blocks the UI. Today `simulate` and `verify` run to the end once
started. The desktop shell already runs the `api` command off the main thread,
but the request holds the document lock, so nothing else can ask it to stop
or ask how far it is.

## Decision

A `Control` (in `cndt-sim`, re-exported by `cndt-api` as `JobControl`) is a
shared cancel flag and a step counter. `Api::job_control()` hands out a clone
that can be kept where another thread reaches it without the lock.

- The simulation checks the flag before every program step and reports steps
  done and steps in all. A cancelled run returns `SimError::Cancelled`.
- A cancelled `simulate` or `verify` request fails with the error kind
  `cancelled`, writes nothing to the document and keeps no partial result.
  Verification never turns a cancel into a "problem"; it returns no report.
- Every request starts with the flag cleared, so a cancel only reaches the
  work running when it was sent.
- The desktop shell exposes `job_cancel` and `job_progress`, which do not take
  the document lock.
- A run that is not cancelled is the same code as before: its result is
  identical to `simulate`'s.

## Options considered

- **Move the API onto a worker thread with a job table and polling.** The
  document, the kernel and the plan caches are not shareable, so every
  request would still need one lock; it adds a protocol without making
  cancel any faster. Not chosen now.
- **Cancel flag only inside the simulation (chosen).** The simulation is the
  work that takes minutes at a fine grid; the shell already runs it off the
  UI thread.

## Consequences

- Cancel is prompt for simulation and verification (one program step).
- Rebuilds, toolpath planning and kernel calls are not interruptible yet;
  they are short next to a fine-grid simulation. Left as a follow-up task.
- No safety check, tolerance or golden file changes.
