# Safety

CNDT produces programs that move real machines. Wrong output can break tools, wreck machines and injure people. Output safety is a release gate, not a best effort.

## For users

- Do not run output from a preview release on a machine.
- Always simulate and review a program before running it.
- Prove out new programs carefully: reduced feed, single block, hand on feed hold.
- Check which post you used. A post marked **unverified** has never been run on a real controller by the project.
- The software has no warranty. You are responsible for what your machine does.

## Gates

1. No operation type is marked production-ready until it has cut a test part on a real machine.
2. No post is marked verified until its output has run on that controller. The record names the machine, the controller, the date and the person.
3. A change that alters any golden G-code file needs explicit review of every difference.
4. Posting with unresolved verification errors needs an explicit user override. The override is recorded in the program header.
5. Every program header states the post name, post version, application version and verification status.
6. Preview releases show a not-for-production notice in the app and in every posted program.

## Verification before posting

Each setup produces a report covering:

- gouges against the target model
- collisions of tool, shank and holder with stock and fixtures
- rapid moves through material
- moves outside the machine's travel limits

What is checked today (decisions 0015 and 0016): rapid moves through material, shanks rubbing above the flutes and holders hitting the stock, and gouges against the exact part, from a stock simulation of each setup; material left; the machine's spindle speed, tool numbers, rigid tapping, rotary steps and the program's extent against its travel; and whether the post accepts the program. Shank and holder checks use each tool's measured assembly, or typical sizes for its kind until it is measured, which the report flags for review. Not yet: fixture and spindle collisions, and travel against the machine's actual coordinates.

`post` verifies first. Rapids through material, broken machine limits and a program the post refuses are **problems**: the program is refused unless the user overrides them, and the override is listed in the program header (gate 4). Gouges, material left, notes and unverified definitions are listed for **review** and do not block. The app saves the report as `<program>.report.txt` beside the G-code.

## Known gap

Early versions simulate the internal toolpath, not the posted G-code. A post bug can therefore pass simulation. Verified posts and golden-file tests cover this until simulation of posted G-code exists.

## Rules for contributors

- Never weaken a verification check, tolerance or test to make something pass.
- Anything that emits G-code has a golden-file test. Regenerate golden files only with `CNDT_UPDATE_GOLDEN=1 cargo test -p cndt-post`, and only after reviewing every changed line.
- Defaults must be conservative. When a value is unknown, choose the safer one or refuse.
- A toolpath generator that cannot produce a safe result fails with a reason. It never emits a partial or best-guess path.
- Report a suspected unsafe-output bug as high priority, with the part, the post and the program.
