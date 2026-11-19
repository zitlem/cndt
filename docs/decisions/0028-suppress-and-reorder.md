# 0028: Suppress and reorder features

- **Status:** Accepted (by agent, 2026-11-19: a flag and a command on the existing timeline; structural dependencies are checked)
- **Date:** 2026-11-18

## Context

The specification's v1.0 history column lists suppressing features and reordering the timeline. Designers switch a fillet or a pattern off to simplify a part, test a variant, or find which feature breaks a later one, and move a feature earlier or later to change what it acts on. Today the only choices are to delete a feature (and lose its settings) or to leave it.

## Decision

- **Suppress:** a timeline entry gets a `suppressed` flag (default false, left out of the file when false). A suppressed feature stays in the timeline with its id and settings but adds nothing to the part. Its status is `suppressed`, not a failure. Names are made from feature ids, so the faces of the other features keep their names.
- **Dependents fail with the reason:** a pattern, circular pattern or mirror of a suppressed feature fails with "it repeats X, which is suppressed". Later features that pick faces the suppressed feature would have made fail as they do for any lost face, saying which face is missing. The rest of the timeline still builds.
- **Reorder:** `feature.move` puts a feature at a new position. It is refused, with nothing changed, when the new order would put a pattern, circular pattern or mirror before the feature it repeats, or a feature before the feature that makes the face its sketch lies on. Other orders are allowed even if a later rebuild fails: a failed feature explains itself and can be moved back.
- **Commands:** `feature.suppress` (`feature`, `suppressed` default true) and `feature.move` (`feature`, `position`). Both undo.

## Options considered

- **A "rollback bar" instead of reordering.** Already served by showing the part at an earlier timeline point; it does not change what features act on.
- **Suppressing a feature and everything that depends on it automatically.** Hidden changes to other features; the explicit failure message is clearer.
- **Refusing every reorder that could break a later feature.** Needs a full rebuild per attempt and blocks legitimate experiments; only structural dependencies are refused.

## Consequences

- The stored format changes: file version 26. Older builds refuse such projects with the update message.
- A suppressed hole is skipped by drilling operations like a hole that does not build.
- Tests: a suppressed feature leaves the part and its names, a pattern of it fails with the reason, unsuppress restores the part, undo, reorder refusals, and the format round trip and version refusal.
